//! Owner-bound OAuth and per-asset lifecycle for Meta text inboxes.
use bson::doc;
use chrono::{Duration, Utc};
use futures::TryStreamExt;
use serde::Serialize;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use super::{
    channel_adapters::meta::{self, Inbox, MetaAdapter, graph_json},
    channel_platform::{BotCredentials, PlatformAdapter, PlatformVerifySecrets},
};
use crate::{
    errors::{AppError, AppResult},
    models::{
        channel_bot::{COLLECTION_NAME as BOTS, ChannelBot, MetaCredentialBinding},
        channel_meta::{AssetCredential, OAuthSession, SESSIONS},
    },
};

pub struct MetaService<'a> {
    pub db: &'a mongodb::Database,
    pub keys: &'a crate::crypto::aes::EncryptionKeys,
    pub http: &'a reqwest::Client,
    pub endpoints: MetaEndpoints,
    pub config: &'a crate::config::AppConfig,
}
impl<'a> From<&'a crate::AppState> for MetaService<'a> {
    fn from(s: &'a crate::AppState) -> Self {
        Self {
            db: &s.db,
            keys: &s.encryption_keys,
            http: &s.http_client,
            endpoints: MetaEndpoints::default(),
            config: &s.config,
        }
    }
}

#[derive(Clone)]
pub struct MetaEndpoints {
    pub facebook_graph: String,
    pub instagram_graph: String,
    pub instagram_oauth: String,
}
impl Default for MetaEndpoints {
    fn default() -> Self {
        Self {
            facebook_graph: "https://graph.facebook.com".into(),
            instagram_graph: "https://graph.instagram.com".into(),
            instagram_oauth: "https://api.instagram.com".into(),
        }
    }
}

#[derive(Serialize)]
pub struct Started {
    pub session_id: String,
    pub authorization_url: String,
}
impl std::fmt::Debug for Started {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MetaStarted([REDACTED])")
    }
}
#[derive(Serialize)]
pub struct Asset {
    pub id: String,
    pub name: String,
}
#[derive(Serialize)]
pub struct Progress {
    pub session_id: String,
    pub platform: String,
    pub status: String,
    pub bot_id: Option<String>,
    pub selected_asset_id: Option<String>,
    pub subscription_state: Option<String>,
    pub assets: Vec<Asset>,
}

pub fn adapter(platform: &str) -> AppResult<MetaAdapter> {
    Ok(MetaAdapter::new(match platform {
        "messenger" => Inbox::Messenger,
        "instagram" => Inbox::Instagram,
        _ => return Err(AppError::NotFound("Unknown inbox platform".into())),
    }))
}
fn conflict() -> AppError {
    AppError::Conflict("The connection changed or expired. Start a new connection attempt.".into())
}
fn fingerprint(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))
}
/// Keep a prepared bot's credential evidence in sync with every provider
/// token replacement. A concurrent storage rewrap forces the caller to reload.
fn credential_update(
    bot: &ChannelBot,
    mut filter: bson::Document,
    mut set: bson::Document,
    mut unset: bson::Document,
) -> AppResult<(bson::Document, bson::Document)> {
    filter.extend(super::channel_credential_continuity::physical_filter(bot)?);
    super::channel_credential_continuity::replacement(bot, &mut set, &mut unset)?;
    Ok((filter, doc! {"$set":set, "$unset":unset}))
}

fn binary(bytes: Vec<u8>) -> bson::Bson {
    bson::Bson::Binary(bson::Binary {
        subtype: bson::spec::BinarySubtype::Generic,
        bytes,
    })
}

async fn require_platform_delivery(
    db: &mongodb::Database,
    tx: &mut mongodb::ClientSession,
    platform: &str,
) -> AppResult<()> {
    if db
        .collection::<bson::Document>(meta::CONTROLS)
        .find_one(doc! {"identity_key": format!("platform:{platform}"), "suspended": true})
        .session(&mut *tx)
        .await?
        .is_some()
    {
        return Err(meta::unreachable("platform_suspended"));
    }
    Ok(())
}

pub(crate) async fn set_platform_delivery(
    db: &mongodb::Database,
    platform: &str,
    enabled: bool,
) -> AppResult<()> {
    super::transaction_service::run_db_only(db, &(db, platform, enabled), |tx, &(db, platform, enabled)| Box::pin(async move {
        super::feature_flag_service::fence_meta_enrollment_in_session(db, platform, tx).await?;
        db.collection::<bson::Document>(meta::CONTROLS).update_one(
            doc! {"identity_key": format!("platform:{platform}")},
            doc! {"$set": {"suspended": !enabled, "at": bson::DateTime::now()},
                  "$setOnInsert": {"_id": uuid::Uuid::new_v4().to_string()}},
        ).upsert(true).session(&mut *tx).await?;
        if !enabled {
            db.collection::<ChannelBot>(BOTS).update_many(
                doc! {"platform": platform, "is_active": true,
                      "meta_binding.subscription": {"$in": ["active", "pending"]}},
                doc! {"$inc": {"meta_binding.generation":1_i64}, "$set": {
                    "status":"suspended", "webhook_registered":false,
                    "meta_binding.subscription":"suspended", "diagnostics.failure_reason":"meta_delivery_suspended",
                    "meta_binding.operator_suspended":true,
                    "updated_at":bson::DateTime::now()}},
            ).session(&mut *tx).await?;
            // An already expired or deauthorized inbox also needs individual
            // resume after the incident. Preserve its credential diagnosis.
            db.collection::<ChannelBot>(BOTS).update_many(
                doc! {"platform": platform, "is_active": true, "meta_binding": {"$type":"object"},
                    "meta_binding.operator_suspended":{"$ne":true}},
                doc! {"$inc":{"meta_binding.generation":1_i64}, "$set":{"meta_binding.operator_suspended":true,
                    "webhook_registered":false, "updated_at":bson::DateTime::now()}},
            ).session(&mut *tx).await?;
        }
        Ok(())
    })).await
}

pub(crate) async fn set_bot_delivery(
    db: &mongodb::Database,
    bot: &ChannelBot,
    actor: &str,
    enabled: bool,
) -> AppResult<()> {
    #[cfg(test)]
    super::durable_wake_tests::after("meta_delivery_before_commit").await;
    let expected = bot.meta_binding.as_ref().ok_or_else(conflict)?.generation;
    super::transaction_service::run_db_only(
        db,
        &(db, bot, actor, enabled, expected),
        |tx, &(db, bot, actor, enabled, expected)| {
            Box::pin(async move {
                MetaService::fence_installation_authority(
                    db,
                    tx,
                    actor,
                    &bot.user_id,
                    &bot.platform,
                    false,
                )
                .await?;
                let bots = db.collection::<ChannelBot>(BOTS);
                let filter = doc! {"_id": &bot.id, "user_id": &bot.user_id, "is_active": true,
                "meta_binding.generation": expected};
                let current = bots
                    .find_one(filter.clone())
                    .session(&mut *tx)
                    .await?
                    .ok_or_else(conflict)?;
                let binding = current.meta_binding.as_ref().ok_or_else(conflict)?;
                if enabled {
                    scopes::fence_current(db, tx, &current).await?;
                    require_platform_delivery(db, tx, &current.platform).await?;
                    meta::require_resumable(&current)?;
                    if binding.subscription != "suspended" {
                        return Ok(());
                    }
                } else if binding.operator_suspended {
                    return Ok(());
                }
                let mut fields = doc! {
                    "status": if enabled {"pending_webhook"} else {"suspended"},
                    "webhook_registered": false,
                    "meta_binding.subscription": if enabled {"pending"} else {"suspended"},
                    "meta_binding.operator_suspended": !enabled,
                    "updated_at": bson::DateTime::now(),
                };
                if !enabled {
                    if binding.subscription == "reconnect_required"
                        || binding.grant_reconnect_required
                        || binding.provider_revoked
                    {
                        fields.insert("meta_binding.grant_reconnect_required", true);
                        fields.insert("meta_binding.subscription", "reconnect_required");
                    }
                    fields.insert("diagnostics.failure_reason", "meta_delivery_suspended");
                }
                if bots
                    .update_one(
                        filter,
                        doc! {"$inc": {"meta_binding.generation": 1_i64}, "$set": fields},
                    )
                    .session(&mut *tx)
                    .await?
                    .matched_count
                    != 1
                {
                    return Err(conflict());
                }
                Ok(())
            })
        },
    )
    .await
}
pub(crate) async fn fence_setup_link(
    db: &mongodb::Database,
    tx: &mut mongodb::ClientSession,
    s: &OAuthSession,
) -> AppResult<()> {
    let Some(id) = &s.connect_link_id else {
        return Ok(());
    };
    let link = db
        .collection::<crate::models::channel_connect_link::ChannelConnectLink>(
            crate::models::channel_connect_link::COLLECTION_NAME,
        )
        .find_one_and_update(
            doc! {"_id":id,
            "connection_id":&s.id,
            "user_id":&s.owner_id,
            "platform":&s.platform,
            "status":{"$in":["pending",
            "completed"]}},
            doc! {"$inc":{"meta_completion_fence":1_i64}},
        )
        .return_document(mongodb::options::ReturnDocument::After)
        .session(&mut *tx)
        .await?
        .ok_or_else(conflict)?;
    use crate::models::channel_connect_link::LinkStatus;
    if (link.status == LinkStatus::Pending
        && super::channel_connect_link_service::effective_deadline(&link) <= Utc::now())
        || (link.status == LinkStatus::Completed && link.bot_id != s.bot_id)
    {
        return Err(conflict());
    }
    Ok(())
}

impl MetaService<'_> {
    fn adapter(&self, platform: &str) -> AppResult<MetaAdapter> {
        let mut a = adapter(platform)?;
        a.graph_base = match a.inbox {
            Inbox::Messenger => self.endpoints.facebook_graph.clone(),
            Inbox::Instagram => self.endpoints.instagram_graph.clone(),
        };
        Ok(a)
    }
    async fn secrets(&self, platform: &str) -> AppResult<PlatformVerifySecrets> {
        let descriptor = adapter(platform)?
            .platform_credentials()
            .ok_or_else(conflict)?;
        let row = super::platform_credential_service::load(self.db, &descriptor).await?;
        if !super::platform_credential_service::configured(row.as_ref(), &descriptor) {
            return Err(super::channel_managed::unavailable());
        }
        super::platform_credential_service::decrypt_snapshot(self.keys, row.as_ref()).await
    }
    async fn live_actor(&self, actor: &str, owner: &str) -> AppResult<()> {
        if self
            .db
            .collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
            .find_one(doc! {"_id":actor,"is_active":true})
            .await?
            .is_none()
        {
            return Err(AppError::Forbidden(
                "Sign in with an active human account".into(),
            ));
        }
        match super::org_service::resolve_owner_access(self.db, actor, owner).await? {
            super::org_service::OwnerAccess::Direct
            | super::org_service::OwnerAccess::AsOrgAdmin { .. } => Ok(()),
            _ => Err(AppError::Forbidden(
                "Inbox management requires the owner or an organization admin".into(),
            )),
        }
    }

    /// Acquire the same authority rows that account disable, departure, and
    /// membership changes fence.  This check deliberately runs inside the
    /// installation transaction, after the provider work, so a demoted org
    /// administrator cannot commit a stale OAuth result.
    pub(crate) async fn fence_installation_authority(
        db: &mongodb::Database,
        tx: &mut mongodb::ClientSession,
        actor: &str,
        owner: &str,
        platform: &str,
        new_installation: bool,
    ) -> AppResult<()> {
        // Built-in Meta enrollment flags have no definition row.  Contend on
        // their shared platform fence so flag writes and this commit have one
        // serial order across every actor.
        super::feature_flag_service::fence_meta_enrollment_in_session(db, platform, tx).await?;
        if new_installation {
            require_platform_delivery(db, tx, platform).await?;
        }
        super::channel_bot_service::fence_owner_management(db, tx, actor, owner).await?;
        if new_installation {
            super::channel_adapters::meta_managed::require_enrollment(db, actor, platform).await?;
        }
        Ok(())
    }
    pub async fn start(
        &self,
        actor: &str,
        owner: &str,
        platform: &str,
        label: &str,
        reconnect: Option<&str>,
    ) -> AppResult<Started> {
        let existing = if let Some(id) = reconnect {
            Some(super::channel_bot_service::get_bot_for_user(self.db, id, owner).await?)
        } else {
            None
        };
        if existing.as_ref().is_some_and(|bot| {
            bot.meta_binding
                .as_ref()
                .is_some_and(|binding| binding.erasure_requested)
        }) {
            return Err(AppError::Conflict("This inbox has a pending provider deletion. Wait for its deletion status to complete, then connect it as a new installation.".into()));
        }
        if let Err(error) =
            super::channel_adapters::meta_managed::require_enrollment(self.db, actor, platform)
                .await
            && existing
                .as_ref()
                .is_none_or(|bot| bot.platform != platform || !bot.is_active)
        {
            return Err(error);
        }
        self.live_actor(actor, owner).await?;
        if label.trim().is_empty() || label.len() > 128 {
            return Err(AppError::ValidationError(
                "Label must be between 1 and 128 characters".into(),
            ));
        }
        let a = self.adapter(platform)?;
        let descriptor = a.platform_credentials().ok_or_else(conflict)?;
        if existing
            .as_ref()
            .is_some_and(|b| b.platform != platform || !b.is_active)
        {
            return Err(conflict());
        }
        let state = Zeroizing::new(hex::encode(rand::random::<[u8; 32]>()));
        let redirect = format!(
            "{}/channel-bots/meta-callback/{platform}",
            self.config.frontend_url.trim_end_matches('/')
        );
        let session = OAuthSession {
            id: uuid::Uuid::new_v4().to_string(),
            state_hash: fingerprint(&state),
            actor_id: actor.into(),
            owner_id: owner.into(),
            platform: platform.into(),
            app_id: String::new(),
            redirect_uri: redirect.clone(),
            label: label.trim().into(),
            status: "pending".into(),
            bot_id: Some(
                existing
                    .as_ref()
                    .map(|b| b.id.clone())
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            ),
            reconnect_bot_id: reconnect.map(str::to_owned),
            expected_generation: existing.and_then(|b| b.meta_binding.map(|m| m.generation)),
            connect_link_id: None,
            grant_subject_id: None,
            selected_asset_id: None,
            assets_encrypted: None,
            created_at: Utc::now(),
            expires_at: Utc::now() + Duration::minutes(15),
        };
        let session = super::transaction_service::run_db_only(
            self.db,
            &(self.db, session, descriptor),
            |tx, (db, template, descriptor)| {
                Box::pin(async move {
                    Self::fence_installation_authority(
                        db,
                        tx,
                        &template.actor_id,
                        &template.owner_id,
                        &template.platform,
                        template.reconnect_bot_id.is_none(),
                    )
                    .await?;
                    if let Some(bot_id) = template.reconnect_bot_id.as_deref() {
                        let bot = db.collection::<ChannelBot>(BOTS)
                            .find_one(doc! {"_id":bot_id,"user_id":&template.owner_id,
                                "is_active":true,"meta_binding.generation":template.expected_generation})
                            .session(&mut *tx).await?.ok_or_else(conflict)?;
                        scopes::fence_reconnect(db, tx, &bot).await?;
                    }
                    use crate::models::platform_credential::{COLLECTION_NAME, PlatformCredential};
                    let row = db
                        .collection::<PlatformCredential>(COLLECTION_NAME)
                        .find_one(doc! {"provider": &template.platform})
                        .session(&mut *tx)
                        .await?;
                    if !super::platform_credential_service::configured_record(
                        row.as_ref(),
                        descriptor,
                    ) {
                        return Err(super::channel_managed::unavailable());
                    }
                    let mut session = template.clone();
                    session.app_id = row
                        .and_then(|row| row.fields.get("app_id").cloned())
                        .ok_or_else(conflict)?;
                    db.collection::<OAuthSession>(SESSIONS)
                        .insert_one(&session)
                        .session(&mut *tx)
                        .await?;
                    Ok(session)
                })
            },
        )
        .await?;
        let (authorize, scopes) = match a.inbox {
            Inbox::Messenger => (
                format!(
                    "https://www.facebook.com/{}/dialog/oauth",
                    meta::GRAPH_VERSION
                ),
                "pages_show_list,pages_manage_metadata,pages_messaging",
            ),
            Inbox::Instagram => (
                "https://www.instagram.com/oauth/authorize".into(),
                "instagram_business_basic,instagram_business_manage_messages",
            ),
        };
        let mut url = reqwest::Url::parse(&authorize).map_err(|_| conflict())?;
        url.query_pairs_mut().extend_pairs([
            ("client_id", session.app_id.as_str()),
            ("redirect_uri", &redirect),
            ("response_type", "code"),
            ("scope", scopes),
            ("state", &state),
        ]);
        if matches!(a.inbox, Inbox::Instagram) {
            url.query_pairs_mut()
                .append_pair("enable_fb_login", "0")
                .append_pair("force_authentication", "1");
        }
        Ok(Started {
            session_id: session.id,
            authorization_url: url.into(),
        })
    }
    async fn session(&self, actor: &str, id: &str) -> AppResult<OAuthSession> {
        let session = self
            .db
            .collection::<OAuthSession>(SESSIONS)
            .find_one(doc! {"_id":id,"actor_id":actor,"expires_at":{"$gt":bson::DateTime::now()}})
            .await?
            .ok_or_else(conflict)?;
        self.live_actor(actor, &session.owner_id).await?;
        Ok(session)
    }
    async fn assets(&self, session: &OAuthSession) -> AppResult<Vec<AssetCredential>> {
        let subject = session
            .grant_subject_id
            .as_deref()
            .filter(|subject| !subject.is_empty())
            .ok_or_else(conflict)?;
        scopes::check_subject_cutoff(
            self.db,
            &session.platform,
            &session.app_id,
            subject,
            session.created_at,
            true,
        )
        .await?;
        let bytes = self
            .keys
            .decrypt(session.assets_encrypted.as_deref().ok_or_else(conflict)?)
            .await?;
        serde_json::from_slice(&bytes).map_err(|_| conflict())
    }
    pub async fn progress(&self, actor: &str, id: &str) -> AppResult<Progress> {
        let session = self.session(actor, id).await?;
        let subscription_state = if let Some(bot_id) = &session.bot_id {
            self.db
                .collection::<ChannelBot>(BOTS)
                .find_one(doc! {"_id": bot_id, "user_id": &session.owner_id, "platform": &session.platform,
                    "meta_binding.app_id": &session.app_id,
                    "meta_binding.grant_id": &session.id})
                .await?
                .and_then(|bot| bot.meta_binding.map(|binding| {
                    if binding.provider_revoked || binding.grant_reconnect_required {
                        "reconnect_required".into()
                    } else {
                        binding.subscription
                    }
                }))
        } else {
            None
        };
        let assets = if session.status == "assets" {
            self.assets(&session)
                .await?
                .into_iter()
                .map(|a| Asset {
                    id: a.id,
                    name: a.name,
                })
                .collect()
        } else {
            Vec::new()
        };
        Ok(Progress {
            session_id: session.id,
            platform: session.platform,
            status: session.status,
            bot_id: session.bot_id,
            selected_asset_id: session.selected_asset_id,
            subscription_state,
            assets,
        })
    }
    pub async fn exchange(
        &self,
        actor: &str,
        platform: &str,
        state: &str,
        code: Option<&str>,
    ) -> AppResult<Progress> {
        if code.is_some_and(|s| s.is_empty() || s.len() > 8192) {
            return Err(AppError::ValidationError(
                "Invalid OAuth authorization code".into(),
            ));
        }
        let session = self
            .db
            .collection::<OAuthSession>(SESSIONS)
            .find_one(doc! {"state_hash":fingerprint(state),
            "actor_id":actor,
            "platform":platform,
            "expires_at":{"$gt":bson::DateTime::now()}})
            .await?
            .ok_or_else(conflict)?;
        if let Err(error) =
            super::channel_adapters::meta_managed::require_enrollment(self.db, actor, platform)
                .await
            && session.reconnect_bot_id.is_none()
        {
            return Err(error);
        }

        self.live_actor(actor, &session.owner_id).await?;
        if session.status != "pending" {
            return self.progress(actor, &session.id).await;
        }
        if self
            .db
            .collection::<OAuthSession>(SESSIONS)
            .update_one(
                doc! {"_id":&session.id,"status":"pending"},
                doc! {"$set":{"status":if code.is_some() {"exchanging"} else {"cancelled"}}},
            )
            .await?
            .modified_count
            != 1
        {
            return Err(conflict());
        }
        let Some(code) = code.filter(|s| !s.is_empty() && s.len() <= 8192) else {
            return self.progress(actor, &session.id).await;
        };
        let result = self.exchange_assets(&session, code).await;
        match result {
            Ok(assets) => {
                if assets.is_empty() {
                    self.db
                        .collection::<OAuthSession>(SESSIONS)
                        .update_one(
                            doc! {"_id":&session.id,"status":"exchanging"},
                            doc! {"$set":{"status":"no_eligible_assets"}},
                        )
                        .await?;
                } else {
                    let subject = assets[0].grant_subject_id.clone();
                    let bytes =
                        Zeroizing::new(serde_json::to_vec(&assets).map_err(|_| conflict())?);
                    let encrypted = self.keys.encrypt(&bytes).await?;
                    let args = (self.db.clone(), session.clone(), subject, encrypted);
                    let stored = super::transaction_service::run_db_only(
                        self.db,
                        &args,
                        |tx, (db, s, subject, encrypted)| {
                            Box::pin(async move {
                                fence_grant_in_session(
                                    db,
                                    tx,
                                    &s.platform,
                                    &s.app_id,
                                    subject,
                                    s.created_at,
                                )
                                .await?;
                                let result = db
                                    .collection::<OAuthSession>(SESSIONS)
                                    .update_one(
                                        doc! {"_id":&s.id,
                                        "status":"exchanging",
                                        "expires_at":{"$gt":bson::DateTime::now()}},
                                        doc! {"$set":{"status":"assets",
                                        "grant_subject_id":subject,
                                        "grant_identity":provider::grant_identity(&s.platform,&s.app_id,subject)?,
                                        "assets_encrypted":binary(encrypted.clone())}},
                                    )
                                    .session(&mut *tx)
                                    .await?;

                                if result.matched_count != 1 {
                                    return Err(conflict());
                                }
                                Ok(())
                            })
                        },
                    )
                    .await;
                    if let Err(error) = stored {
                        self.db
                            .collection::<OAuthSession>(SESSIONS)
                            .update_one(
                                doc! {"_id":&session.id,"status":"exchanging"},
                                doc! {"$set":{"status":"revoked"},"$unset":{"assets_encrypted":""}},
                            )
                            .await?;
                        return Err(error);
                    }
                }
            }
            Err(e) => {
                self.db
                    .collection::<OAuthSession>(SESSIONS)
                    .update_one(
                        doc! {"_id":&session.id,"status":"exchanging"},
                        doc! {"$set":{"status":"failed"}},
                    )
                    .await?;
                return Err(e);
            }
        }
        self.progress(actor, &session.id).await
    }
    async fn exchange_assets(
        &self,
        s: &OAuthSession,
        code: &str,
    ) -> AppResult<Vec<AssetCredential>> {
        let a = self.adapter(&s.platform)?;
        let secrets = self.secrets(&s.platform).await?;
        if secrets.get("app_id") != Some(s.app_id.as_str()) {
            return Err(conflict());
        }
        let secret = secrets.get("client_secret").ok_or_else(conflict)?;
        let endpoint = match a.inbox {
            Inbox::Messenger => a.url("oauth/access_token"),
            Inbox::Instagram => format!("{}/oauth/access_token", self.endpoints.instagram_oauth),
        };
        let short = graph_json(self.http.post(endpoint).form(&[
            ("client_id", s.app_id.as_str()),
            ("client_secret", secret),
            ("redirect_uri", &s.redirect_uri),
            ("code", code),
            ("grant_type", "authorization_code"),
        ]))
        .await?;
        let short_grant = match a.inbox {
            Inbox::Messenger => &short,
            Inbox::Instagram => {
                let grant = meta::instagram_record(&short)?;
                let permissions = grant["permissions"].as_str().ok_or_else(|| {
                    AppError::ValidationError(
                        "Instagram consent did not report its granted permissions".into(),
                    )
                })?;
                for required in [
                    "instagram_business_basic",
                    "instagram_business_manage_messages",
                ] {
                    if !permissions
                        .split(',')
                        .map(str::trim)
                        .any(|granted| granted == required)
                    {
                        return Err(AppError::ValidationError("Instagram consent is missing required permissions; reconnect and grant inbox access".into()));
                    }
                }
                grant
            }
        };
        let short_subject = if matches!(a.inbox, Inbox::Instagram) {
            Some(meta::identity_value(&short_grant["user_id"]).ok_or_else(conflict)?)
        } else {
            None
        };
        let short_token = Zeroizing::new(
            short_grant["access_token"]
                .as_str()
                .filter(|token| !token.is_empty())
                .ok_or_else(conflict)?
                .to_owned(),
        );
        let long = match a.inbox {
            Inbox::Messenger => {
                graph_json(self.http.post(a.url("oauth/access_token")).form(&[
                    ("grant_type", "fb_exchange_token"),
                    ("client_id", s.app_id.as_str()),
                    ("client_secret", secret),
                    ("fb_exchange_token", short_token.as_str()),
                ]))
                .await?
            }
            Inbox::Instagram => {
                graph_json(
                    self.http
                        .get(format!("{}/access_token", self.endpoints.instagram_graph))
                        .query(&[
                            ("grant_type", "ig_exchange_token"),
                            ("client_secret", secret),
                            ("access_token", short_token.as_str()),
                        ]),
                )
                .await?
            }
        };
        let token = Zeroizing::new(
            long["access_token"]
                .as_str()
                .ok_or_else(conflict)?
                .to_owned(),
        );
        let expiry = long["expires_in"]
            .as_i64()
            .filter(|n| *n > 0 && *n <= 366 * 86400)
            .map(|n| Utc::now() + Duration::seconds(n));
        match a.inbox {
            Inbox::Instagram => {
                let identity = a
                    .verify_bot_token(self.http, &BotCredentials::from(token.as_str()))
                    .await?;
                // Instagram Login returns a login ID separately; /me.user_id is
                // the professional inbox ID used by signed webhook entry.id.
                Ok(vec![AssetCredential {
                    id: identity.platform_bot_id.clone(),
                    name: identity.platform_bot_username,
                    token,
                    grant_subject_id: short_subject.ok_or_else(conflict)?,
                    expires_at: Some(expiry.ok_or_else(conflict)?),
                }])
            }
            Inbox::Messenger => {
                let me = graph_json(
                    self.http
                        .get(a.url("me"))
                        .bearer_auth(token.as_str())
                        .query(&[("fields", "id")]),
                )
                .await?;
                let subject = me["id"]
                    .as_str()
                    .filter(|id| meta::valid_id(id))
                    .ok_or_else(conflict)?;
                let permissions = graph_json(
                    self.http
                        .get(a.url("me/permissions"))
                        .bearer_auth(token.as_str()),
                )
                .await?;
                for needed in [
                    "pages_messaging",
                    "pages_show_list",
                    "pages_manage_metadata",
                ] {
                    if !permissions["data"].as_array().is_some_and(|rows| {
                        rows.iter()
                            .any(|p| p["permission"] == needed && p["status"] == "granted")
                    }) {
                        return Err(AppError::ValidationError("Facebook consent is missing required Page permissions; reconnect and grant access".into()));
                    }
                }
                let mut assets = Vec::new();
                let mut after = String::new();
                const MAX_PAGE_PAGES: usize = 50;
                let mut exhausted_bound = true;
                for _ in 0..MAX_PAGE_PAGES {
                    let mut request = self
                        .http
                        .get(a.url("me/accounts"))
                        .bearer_auth(token.as_str())
                        .query(&[("fields", "id,name,tasks,access_token"), ("limit", "100")]);
                    if !after.is_empty() {
                        request = request.query(&[("after", after.as_str())]);
                    }
                    let page = graph_json(request).await?;
                    for asset in page["data"].as_array().ok_or_else(conflict)? {
                        if !asset["tasks"].as_array().is_some_and(|tasks| {
                            tasks
                                .iter()
                                .any(|t| matches!(t.as_str(), Some("MESSAGING" | "MANAGE")))
                        }) {
                            continue;
                        }
                        let Some(id) = asset["id"].as_str().filter(|s| meta::valid_id(s)) else {
                            continue;
                        };
                        let Some(page_token) =
                            asset["access_token"].as_str().filter(|s| !s.is_empty())
                        else {
                            continue;
                        };
                        assets.push(AssetCredential {
                            id: id.into(),
                            name: asset["name"].as_str().unwrap_or(id).into(),
                            token: Zeroizing::new(page_token.into()),
                            grant_subject_id: subject.into(),
                            // The long-lived login token's expiry is not the Page
                            // credential's lifetime. Graph revocation is authoritative.
                            expires_at: None,
                        });
                    }
                    let next = page["paging"]["cursors"]["after"]
                        .as_str()
                        .filter(|s| s.len() <= 2048 && !s.is_empty());
                    if page["paging"].get("next").is_none()
                        || next.is_none()
                        || next == Some(after.as_str())
                    {
                        exhausted_bound = false;
                        break;
                    }
                    after = next.unwrap_or_default().into();
                }
                if exhausted_bound {
                    return Err(AppError::ChannelPlatformError(
                        "Meta returned too many Pages to enumerate safely; narrow the account or retry after cleanup".into(),
                    ));
                }
                Ok(assets)
            }
        }
    }
    pub async fn finish(
        &self,
        actor: &str,
        id: &str,
        asset_id: &str,
        exclusive: bool,
    ) -> AppResult<ChannelBot> {
        let s = self.session(actor, id).await?;
        if let Err(error) =
            super::channel_adapters::meta_managed::require_enrollment(self.db, actor, &s.platform)
                .await
            && s.reconnect_bot_id.is_none()
        {
            return Err(error);
        }
        if s.status == "completed" {
            if s.selected_asset_id.as_deref() != Some(asset_id) {
                return Err(conflict());
            }
            self.validate_link(&s).await?;
            let bot = super::channel_bot_service::get_bot_for_user(
                self.db,
                s.bot_id.as_deref().ok_or_else(conflict)?,
                &s.owner_id,
            )
            .await?;
            if !bot.is_active
                || bot
                    .meta_binding
                    .as_ref()
                    .is_none_or(|binding| binding.grant_id != s.id)
            {
                return Err(conflict());
            }
            if !bot
                .meta_binding
                .as_ref()
                .is_some_and(|binding| binding.operator_suspended)
                && (bot.status != "active" || !bot.webhook_registered)
            {
                self.verify(&bot).await?;
            }
            if let Some(link) = &s.connect_link_id {
                super::channel_connect_link_service::complete_meta(self.db, link, &s.id).await?;
            }
            return super::channel_bot_service::get_bot(self.db, &bot.id).await;
        }
        if !exclusive {
            return Err(AppError::ValidationError(
                "Confirm that this inbox uses verified exclusive routing to this app".into(),
            ));
        }
        self.validate_link(&s).await?;
        let asset = self
            .assets(&s)
            .await?
            .into_iter()
            .find(|a| a.id == asset_id)
            .ok_or_else(|| {
                AppError::Forbidden(
                    "Select an asset from this connection's eligible account list".into(),
                )
            })?;
        let a = self.adapter(&s.platform)?;
        let secrets = self.secrets(&s.platform).await?;
        if secrets.get("app_id") != Some(s.app_id.as_str()) {
            return Err(conflict());
        }
        let credentials = BotCredentials {
            billing: None,
            token: &asset.token,
            platform_bot_id: Some(&asset.id),
            platform_secrets: None,
        };
        let identity = a.verify_bot_token(self.http, &credentials).await?;
        if s.status == "installing" {
            if s.selected_asset_id.as_deref() != Some(asset_id) {
                return Err(conflict());
            }
        } else if self
            .db
            .collection::<OAuthSession>(SESSIONS)
            .update_one(
                doc! {"_id":id,
                "status":"assets",
                "actor_id":actor,
                "expires_at":{"$gt":bson::DateTime::now()}},
                doc! {"$set":{"status":"installing",
                "selected_asset_id":asset_id}},
            )
            .await?
            .modified_count
            != 1
        {
            return Err(conflict());
        }
        let binding = MetaCredentialBinding {
            operator_suspended: false,
            grant_reconnect_required: false,
            provider_revoked: false,
            erasure_requested: false,
            grant_id: s.id.clone(),
            authorized_at: Some(s.created_at),
            events_since: Some(Utc::now()),
            generation: s.expected_generation.unwrap_or(0) + 1,
            grant_start_generation: s.expected_generation.unwrap_or(0) + 1,
            token_fingerprint: meta::token_fingerprint(&asset.token),
            grant_actor_id: actor.into(),
            grant_subject_id: asset.grant_subject_id,
            app_id: s.app_id.clone(),
            exclusive_routing_verified: true,
            subscription: "pending".into(),
            next_reconcile_at: None,
            expires_at: asset.expires_at,
        };
        let bot = if let Some(bot_id) = &s.reconnect_bot_id {
            self.reconnect_install(&s, &binding, &asset.id, &asset.token)
                .await?;
            super::channel_bot_service::get_bot(self.db, bot_id).await?
        } else {
            let bot_id = s.bot_id.as_deref().ok_or_else(conflict)?;
            let document = doc! {
                "_id":bot_id,"user_id":&s.owner_id,"ownership_version":0_i64,"platform":&s.platform,"label":&s.label,
                "credential_source":"platform","bot_token_encrypted":binary(self.keys.encrypt(asset.token.as_bytes()).await?),
                "platform_bot_id":&asset.id,"platform_bot_username":&identity.platform_bot_username,"webhook_registered":false,
                "webhook_secret_hash":"","app_id":&s.app_id,"meta_binding":bson::to_bson(&binding).map_err(|_|conflict())?,
                "status":"pending_webhook","is_active":true,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()
            };
            let bot: ChannelBot = bson::from_document(document).map_err(|_| conflict())?;
            super::channel_bot_service::insert_meta_bot(
                self.db,
                &bot,
                self.config.channel_relay_max_bots_per_user,
                &s,
            )
            .await?;
            bot
        };
        if !bot
            .meta_binding
            .as_ref()
            .is_some_and(|binding| binding.operator_suspended)
        {
            self.verify(&bot).await?;
        }
        if let Some(link_id) = &s.connect_link_id {
            super::channel_connect_link_service::complete_meta(self.db, link_id, &s.id).await?;
        }
        super::channel_bot_service::get_bot(self.db, &bot.id).await
    }
    async fn validate_link(&self, s: &OAuthSession) -> AppResult<()> {
        if s.connect_link_id.is_none() {
            return Ok(());
        }
        let args = (self.db.clone(), s.clone());
        super::transaction_service::run_db_only(self.db, &args, |tx, (db, s)| {
            Box::pin(async move { fence_setup_link(db, tx, s).await })
        })
        .await
    }
    async fn reconnect_install(
        &self,
        s: &OAuthSession,
        binding: &MetaCredentialBinding,
        asset: &str,
        token: &str,
    ) -> AppResult<()> {
        let encrypted = self.keys.encrypt(token.as_bytes()).await?;
        let args = (
            self.db.clone(),
            s.clone(),
            binding.clone(),
            asset.to_owned(),
            encrypted,
        );
        super::transaction_service::run_db_only(self.db, &args, |tx, (db, s, binding, asset, encrypted)| {
            Box::pin(async move {
                let session = db.collection::<OAuthSession>(SESSIONS).find_one(doc! {
                    "_id": &s.id, "actor_id": &s.actor_id, "selected_asset_id": asset,
                    "expires_at": {"$gt": bson::DateTime::now()},
                }).session(&mut *tx).await?.ok_or_else(conflict)?;
                if session.status == "completed" { return Ok(()); }
                if session.status != "installing" { return Err(conflict()); }
                Self::fence_installation_authority(
                    db,
                    tx,
                    &session.actor_id,
                    &session.owner_id,
                    &session.platform,
                    false,
                )
                .await?;
                fence_grant_in_session(db, tx, &s.platform, &s.app_id, &binding.grant_subject_id, s.created_at).await?;
                fence_setup_link(db, tx, s).await?;
                let bot_id = s.reconnect_bot_id.as_deref().ok_or_else(conflict)?;
                let old = db.collection::<ChannelBot>(BOTS).find_one(doc! {
                    "_id": bot_id, "user_id": &s.owner_id, "is_active": true,
                    "platform_bot_id": asset, "meta_binding.generation": s.expected_generation,
                }).session(&mut *tx).await?.ok_or_else(conflict)?;
                if old.credential_source != "platform" {
                    super::channel_signing_migration::registration_fence(db, tx).await?;
                }
                let preserve_suspension = old
                    .meta_binding
                    .as_ref()
                    .is_some_and(|old_binding| old_binding.operator_suspended || old_binding.subscription == "suspended");
                let mut binding = binding.clone();
                binding.events_since = Some(Utc::now());
                binding.operator_suspended = old
                    .meta_binding
                    .as_ref()
                    .is_some_and(|old_binding| old_binding.operator_suspended);
                if preserve_suspension {
                    binding.operator_suspended = true;
                    binding.subscription = "suspended".into();
                }
                scopes::cutover(db,tx,&old,&binding).await?;
                // An address verified through A is not independent proof for
                // B. Retain the assignment and policy; require a fresh owner
                // challenge on the current active connection after consent.
                db.collection::<bson::Document>(crate::models::nyxbot_channel::COLLECTION_NAME)
                    .update_one(doc! {"channel_bot_id":bot_id,"status":"active"},
                        doc! {"$set":{"owner_sender_ids":[]},"$unset":{"link_code_hash":"","link_code_expires_at":""},
                            "$inc":{"execution_policy_revision":1_i64}}).session(&mut *tx).await?;
                // Participant observations remain on their original rows. Their
                // admitted grant/range prevents a prior grant from authorizing B;
                // B replaces a participant row only after fresh provider proof.
                let (filter, update) = credential_update(
                    &old,
                    doc! {"_id": bot_id, "user_id": &s.owner_id, "is_active": true,
                        "meta_binding.generation": s.expected_generation},
                    doc! {
                        "bot_token_encrypted": binary(encrypted.clone()), "app_id": &s.app_id,
                        "credential_source": "platform", "app_secret_encrypted": null,
                        "meta_binding": bson::to_bson(&binding).map_err(|_| conflict())?,
                        "status": if preserve_suspension { "suspended" } else { "pending_webhook" },
                        "webhook_registered": false,
                        "error": bson::Bson::Null,
                        "diagnostics.failure_reason": preserve_suspension.then_some("meta_delivery_suspended"),
                        "updated_at": bson::DateTime::now(),
                    }, doc! {
                        "secret_context_versions.bot_token_encrypted": "",
                        "secret_context_versions.app_secret_encrypted": "",
                    },
                )?;
                let changed = db.collection::<ChannelBot>(BOTS)
                    .update_one(filter, update).session(&mut *tx).await?;
                if changed.matched_count != 1 { return Err(conflict()); }
                if old.credential_source != "platform" {
                    super::channel_signing_migration::prepare_observed(db, tx, bot_id).await?;
                }
                if old.app_id.as_deref() != Some(s.app_id.as_str()) {
                    let mut cursor = db
                        .collection::<crate::models::nyxbot_channel::NyxbotChannel>(
                            crate::models::nyxbot_channel::COLLECTION_NAME,
                        )
                        .find(doc! {"channel_bot_id": bot_id})
                        .session(&mut *tx)
                        .await?;
                    let channels: Vec<crate::models::nyxbot_channel::NyxbotChannel> =
                        cursor.stream(&mut *tx).try_collect().await?;
                    for channel in channels {
                        super::nyxbot_gateway_release_service::retire_references_in_session(
                            db,
                            &channel,
                            "meta_application_changed",
                            tx,
                        )
                        .await?;
                    }
                }
                let done = db.collection::<OAuthSession>(SESSIONS).update_one(
                    doc! {"_id": &s.id, "status": "installing"},
                    doc! {"$set": {"status": "completed", "expires_at": bson::DateTime::from_chrono(Utc::now() + Duration::hours(2))},
                        "$unset": {"assets_encrypted": ""}},
                ).session(&mut *tx).await?;
                if done.matched_count != 1 { return Err(conflict()); }
                Ok(())
            })
        }).await
    }

    pub async fn verify(&self, bot: &ChannelBot) -> AppResult<()> {
        super::channel_connection_webhook_service::serialized(
            self.db,
            &format!("meta-asset:{}:{}", bot.platform, bot.platform_bot_id),
            self.verify_inner(bot),
        )
        .await
    }
    async fn verify_inner(&self, bot: &ChannelBot) -> AppResult<()> {
        let current = super::channel_bot_service::get_bot(self.db, &bot.id).await?;
        if !current.is_active
            || current.meta_binding.as_ref().map(|b| b.generation)
                != bot.meta_binding.as_ref().map(|b| b.generation)
        {
            return Err(conflict());
        }
        scopes::require_current_operation(self.db, &current).await?;
        let result = self.verify_current(&current).await;
        if matches!(&result,Err(AppError::ChannelConversationNotReachable(reason)) if reason == "meta:reconnect_required")
        {
            self.reconnect_required(&current).await?;
        }
        result
    }
    async fn verify_current(&self, bot: &ChannelBot) -> AppResult<()> {
        if bot
            .meta_binding
            .as_ref()
            .is_some_and(|binding| binding.subscription == "suspended")
        {
            return Err(meta::unreachable("platform_suspended"));
        }
        if meta::require_verifiable(bot).is_err() {
            if bot.meta_binding.as_ref().is_some_and(|binding| {
                binding.provider_revoked
                    || binding.grant_reconnect_required
                    || binding.subscription == "reconnect_required"
            }) {
                self.reconnect_required(bot).await?;
            }
            return Err(meta::unreachable("reconnect_required"));
        }
        if self
            .db
            .collection::<bson::Document>(meta::CONTROLS)
            .find_one(doc! {"identity_key":format!("platform:{}",bot.platform),"suspended":true})
            .await?
            .is_some()
        {
            return Err(meta::unreachable("platform_suspended"));
        }
        let a = self.adapter(&bot.platform)?;
        let token =
            super::channel_credentials::resolve_bot_token(self.db, self.keys, &a, bot).await?;
        let credentials = BotCredentials {
            billing: None,
            token: &token,
            platform_bot_id: Some(&bot.platform_bot_id),
            platform_secrets: None,
        };
        if let Err(error) = a.verify_bot_token(self.http, &credentials).await {
            #[cfg(test)]
            super::durable_wake_tests::after("meta_verify_token_rejected").await;
            if matches!(&error, AppError::ChannelConversationNotReachable(reason) if reason == "meta:reconnect_required")
            {
                self.reconnect_required(bot).await?;
            }
            return Err(error);
        }
        let binding = bot.meta_binding.as_ref().ok_or_else(conflict)?;
        let mut secrets = PlatformVerifySecrets::default();
        if bot.credential_source == "platform" {
            secrets = self.secrets(&bot.platform).await?;
        } else {
            secrets.insert("app_id", binding.app_id.clone());
        }
        if secrets.get("app_id") != Some(binding.app_id.as_str())
            || binding.token_fingerprint != meta::token_fingerprint(&token)
        {
            return Err(conflict());
        }
        if matches!(a.inbox, Inbox::Messenger) && bot.credential_source == "platform" {
            // Person-specific Page tasks come from /me/accounts at consent.
            // Use the issuing app to inspect the retained Page capability.
            let app_token = Zeroizing::new(format!(
                "{}|{}",
                binding.app_id,
                secrets.get("client_secret").ok_or_else(conflict)?
            ));
            let inspected = meta::graph_json_read(
                self.http
                    .get(a.url("debug_token"))
                    .bearer_auth(app_token.as_str())
                    .query(&[("input_token", token.as_str())]),
            )
            .await
            .map_err(|_| {
                AppError::ChannelPlatformError("Meta Page token inspection is unavailable".into())
            })?;
            let data = &inspected["data"];
            let scopes = data["scopes"].as_array();
            let authorized = data["is_valid"] == true
                && meta::identity_value(&data["app_id"]).as_deref()
                    == Some(binding.app_id.as_str())
                && data["type"].as_str() == Some("PAGE")
                && data.get("profile_id").is_none_or(|id| {
                    meta::identity_value(id).as_deref() == Some(bot.platform_bot_id.as_str())
                })
                && ["pages_messaging", "pages_manage_metadata"]
                    .iter()
                    .all(|scope| {
                        scopes.is_some_and(|values| values.iter().any(|value| value == scope))
                            && data["granular_scopes"].as_array().is_none_or(|rows| {
                                rows.iter().filter(|row| row["scope"] == *scope).all(|row| {
                                    row.get("target_ids").is_none_or(|ids| {
                                        ids.as_array().is_some_and(|ids| {
                                            ids.iter().any(|id| {
                                                meta::identity_value(id).as_deref()
                                                    == Some(bot.platform_bot_id.as_str())
                                            })
                                        })
                                    })
                                })
                            })
                    });
            if !authorized {
                self.reconnect_required(bot).await?;
                return Err(meta::unreachable("reconnect_required"));
            }
        }
        // Subscription is asset-specific: never remove the person's whole grant.
        let fields = match a.inbox {
            Inbox::Messenger => "messages,messaging_handovers",
            Inbox::Instagram => "messages",
        };
        let subscription_url = a.url(&format!("{}/subscribed_apps", bot.platform_bot_id));
        let matches = |value: &serde_json::Value| {
            value["data"].as_array().is_some_and(|rows| {
                rows.iter().any(|row| {
                    row["id"].as_str() == Some(binding.app_id.as_str())
                        && row["subscribed_fields"].as_array().is_some_and(|fields| {
                            let required = match a.inbox {
                                Inbox::Messenger => ["messages", "messaging_handovers"].as_slice(),
                                Inbox::Instagram => ["messages"].as_slice(),
                            };
                            required
                                .iter()
                                .all(|field| fields.iter().any(|value| value == field))
                        })
                })
            })
        };
        let registered = if binding.subscription == "active" {
            let list =
                meta::graph_json_read(self.http.get(&subscription_url).bearer_auth(token.as_str()))
                    .await?;
            matches(&list)
        } else {
            false
        };
        if !registered {
            if binding.subscription == "active" {
                // Readback proved a missing subscription. Expose that fact even
                // if the subsequent provider repair is interrupted or refused.
                self.publish_verification_state(bot,doc! {"meta_binding.subscription":"active"},
                    doc! {"meta_binding.subscription":"pending","status":"pending_webhook",
                        "webhook_registered":false,"diagnostics.subscription_state":{"registered":false,"checked_at":bson::DateTime::now(),"last_error":null},
                        "meta_binding.next_reconcile_at":bson::DateTime::now()}).await?;
            }
            scopes::require_current_operation(self.db, bot).await?;
            let subscribed = graph_json(
                self.http
                    .post(&subscription_url)
                    .bearer_auth(token.as_str())
                    .form(&[("subscribed_fields", fields)]),
            )
            .await?;
            if subscribed["success"] != true {
                return Err(AppError::ChannelPlatformError(
                    "Meta subscription was not confirmed; select Verify to retry".into(),
                ));
            }
            let list =
                meta::graph_json_read(self.http.get(&subscription_url).bearer_auth(token.as_str()))
                    .await?;
            if !matches(&list) {
                return Err(AppError::ChannelPlatformError(
                    "Meta subscription readback failed; select Verify to retry".into(),
                ));
            }
        }
        let mut verify_fields = doc! {
            "meta_binding.subscription":"active",
            "meta_binding.next_reconcile_at":bson::DateTime::from_chrono(Utc::now()+Duration::days(1)),
            "webhook_registered":true,
            "status":"active",
            "error":null,
            "diagnostics.failure_reason":bson::Bson::Null,
            "diagnostics.subscription_state":{"registered":true,
                "checked_at":bson::DateTime::now(),
                "last_error":null},
            "updated_at":bson::DateTime::now(),
        };
        if bot.platform == "messenger" {
            verify_fields.insert("meta_binding.expires_at", bson::Bson::Null);
        }
        #[cfg(test)]
        super::durable_wake_tests::after("meta_verify_provider_confirmed").await;
        // Instagram verification observes the credential; it does not publish
        // an old expiry over a concurrent same-token lifetime extension.
        self.publish_verification_state(bot, doc! {}, verify_fields)
            .await
    }

    async fn publish_verification_state(
        &self,
        bot: &ChannelBot,
        mut filter: bson::Document,
        fields: bson::Document,
    ) -> AppResult<()> {
        let binding = bot.meta_binding.as_ref().ok_or_else(conflict)?;
        filter.extend(doc! {"_id":&bot.id,"user_id":&bot.user_id,"is_active":true,
            "meta_binding.generation":binding.generation,"meta_binding.token_fingerprint":&binding.token_fingerprint});
        filter.extend(grants::grant_filter(&binding.grant_id));
        super::transaction_service::run_db_only(
            self.db,
            &(self.db, &filter, &fields),
            |tx, &(db, filter, fields)| {
                Box::pin(async move {
                    let live = db
                        .collection::<ChannelBot>(BOTS)
                        .find_one(filter.clone())
                        .session(&mut *tx)
                        .await?
                        .ok_or_else(conflict)?;
                    scopes::fence_current(db, tx, &live).await?;
                    if db
                        .collection::<ChannelBot>(BOTS)
                        .update_one(filter.clone(), doc! {"$set":fields.clone()})
                        .session(tx)
                        .await?
                        .matched_count
                        != 1
                    {
                        return Err(conflict());
                    }
                    Ok(())
                })
            },
        )
        .await
    }
    /// Apply a confirmed provider response to its original grant. Rewrap may
    /// change every envelope during HTTP. Authenticate the token's logical pin
    /// in the transaction that replaces it, so storage retries stay local.
    async fn publish_token_result(
        &self,
        observed: &ChannelBot,
        enabling: bool,
        mut filter: bson::Document,
        set: bson::Document,
    ) -> AppResult<()> {
        use super::channel_credential_continuity::{TOKEN, field_pin, replacement};
        let binding = observed.meta_binding.as_ref().ok_or_else(conflict)?;
        filter.extend(doc! {
            "_id": &observed.id, "user_id": &observed.user_id, "platform": &observed.platform,
            "platform_bot_id": &observed.platform_bot_id,
            "meta_binding.app_id": &binding.app_id, "meta_binding.token_fingerprint": &binding.token_fingerprint,
        });
        filter.extend(grants::grant_filter(&binding.grant_id));
        let expected_pin = field_pin(observed, TOKEN)?;
        let args = (
            self.db,
            self.db.collection::<ChannelBot>(BOTS),
            enabling,
            filter,
            expected_pin,
            observed.ownership_version,
            set,
        );
        super::transaction_service::run_db_only(
            self.db,
            &args,
            |tx, (db, bots, enabling, filter, pin, ownership_version, set)| {
                Box::pin(async move {
                    let live = bots
                        .find_one(filter.clone())
                        .session(&mut *tx)
                        .await?
                        .ok_or_else(conflict)?;
                    if live.ownership_version != *ownership_version
                        || field_pin(&live, TOKEN)? != *pin
                    {
                        return Err(conflict());
                    }
                    if *enabling {
                        scopes::fence_current(db, tx, &live).await?;
                    }
                    let mut set = set.clone();
                    let mut unset = doc! {"secret_context_versions.bot_token_encrypted": ""};
                    replacement(&live, &mut set, &mut unset)?;
                    let result = bots
                        .update_one(doc! {"_id": &live.id}, doc! {"$set": set, "$unset": unset})
                        .session(&mut *tx)
                        .await?;
                    if result.matched_count != 1 {
                        return Err(conflict());
                    }
                    Ok(())
                })
            },
        )
        .await
    }
    pub(crate) async fn delete_snapshot(
        &self,
        actor: &str,
        bot: &ChannelBot,
    ) -> AppResult<Option<&'static str>> {
        super::channel_connection_webhook_service::serialized(
            self.db,
            &format!("meta-asset:{}:{}", bot.platform, bot.platform_bot_id),
            self.delete_snapshot_inner(actor, bot),
        )
        .await
    }
    async fn delete_snapshot_inner(
        &self,
        actor: &str,
        bot: &ChannelBot,
    ) -> AppResult<Option<&'static str>> {
        let grant = &bot.meta_binding.as_ref().ok_or_else(conflict)?.grant_id;
        let stopped = grants::stop_for_delete(self.db, actor, bot).await?;
        #[cfg(test)]
        super::durable_wake_tests::after("meta_delete_after_stop").await;
        let result = self.disconnect_inner(&stopped).await;
        let current = match super::channel_bot_service::get_bot_for_user(
            self.db,
            &bot.id,
            &bot.user_id,
        )
        .await
        {
            Ok(current) => current,
            Err(AppError::ChannelBotNotFound(_))
                if grants::completion_for(self.db, &bot.id, grant).await? =>
            {
                return Ok(Some("removed"));
            }
            Err(error) => return Err(error),
        };
        if current.is_active {
            return Err(result.err().unwrap_or_else(conflict));
        }
        let removed = current
            .meta_binding
            .as_ref()
            .is_some_and(|binding| binding.subscription == "removed");
        self.db
            .collection::<bson::Document>(crate::models::channel_conversation::COLLECTION_NAME)
            .update_many(
                doc! {"channel_bot_id":&bot.id},
                doc! {"$set":{"is_active":false,"updated_at":bson::DateTime::now()}},
            )
            .await?;
        Ok(Some(if removed { "removed" } else { "failed" }))
    }

    pub async fn disconnect(&self, bot: &ChannelBot) -> AppResult<()> {
        super::channel_connection_webhook_service::serialized(
            self.db,
            &format!("meta-asset:{}:{}", bot.platform, bot.platform_bot_id),
            self.disconnect_inner(bot),
        )
        .await
    }
    async fn disconnect_inner(&self, bot: &ChannelBot) -> AppResult<()> {
        let binding = bot.meta_binding.as_ref().ok_or_else(conflict)?;
        let a = self.adapter(&bot.platform)?;
        let current = super::channel_bot_service::get_bot(self.db, &bot.id).await?;
        if current
            .meta_binding
            .as_ref()
            .is_some_and(|binding| binding.subscription == "removed")
        {
            return Ok(());
        }
        let generation = current
            .meta_binding
            .as_ref()
            .ok_or_else(conflict)?
            .generation;
        if current.is_active {
            let result = self
                .db
                .collection::<ChannelBot>(BOTS)
                .update_one(
                    doc! {"_id":&bot.id,
                    "user_id":&bot.user_id,
                    "is_active":true,
                    "meta_binding.generation":binding.generation},
                    doc! {"$inc":{"meta_binding.generation":1},
                    "$set":{"is_active":false,
                    "status":"inactive",
                    "meta_binding.subscription":"removal_pending",
                    "meta_binding.next_reconcile_at":bson::DateTime::now(),
                    "webhook_registered":false,
                    "updated_at":bson::DateTime::now()}},
                )
                .await?;

            if result.modified_count != 1 {
                return Err(conflict());
            }
        }
        let successor = self
            .db
            .collection::<ChannelBot>(BOTS)
            .find_one(doc! {"_id":{"$ne":&bot.id},
            "platform":&bot.platform,
            "platform_bot_id":&bot.platform_bot_id,
            "app_id":&bot.app_id,
            "is_active":true})
            .await?;

        if successor.is_none()
            && !current
                .meta_binding
                .as_ref()
                .is_some_and(|binding| binding.provider_revoked)
        {
            // Decryption/transport failure retains both the encrypted credential
            // and removal_pending. A successor always wins over old cleanup.
            let token = Zeroizing::new(
                super::channel_bot_service::decrypt_bot_token(self.keys, &current).await?,
            );
            match graph_json(
                self.http
                    .delete(a.url(&format!("{}/subscribed_apps", bot.platform_bot_id)))
                    .bearer_auth(token.as_str()),
            )
            .await
            {
                Ok(value) if value["success"] == true => {}
                Err(AppError::ChannelConversationNotReachable(reason))
                    if reason == "meta:reconnect_required" => {}
                Err(error) => return Err(error),
                _ => {
                    return Err(AppError::ChannelPlatformError(
                        "Meta subscription removal is unconfirmed".into(),
                    ));
                }
            }
        }
        #[cfg(test)]
        super::durable_wake_tests::after("meta_disconnect_provider_confirmed").await;
        self.publish_token_result(
            &current,
            false,
            doc! {"is_active":false, "meta_binding.subscription":"removal_pending",
            "meta_binding.generation":if current.is_active {generation+1} else {generation}},
            doc! {"meta_binding.subscription":"removed", "bot_token_encrypted":binary(Vec::new()),
            "updated_at":bson::DateTime::now()},
        )
        .await
    }
    pub async fn refresh(&self, bot: &ChannelBot) -> AppResult<()> {
        if bot.platform != "instagram" {
            return Ok(());
        }
        super::channel_connection_webhook_service::serialized(
            self.db,
            &format!("meta-asset:{}:{}", bot.platform, bot.platform_bot_id),
            self.refresh_guarded(bot),
        )
        .await
    }

    async fn refresh_guarded(&self, bot: &ChannelBot) -> AppResult<()> {
        let current = super::channel_bot_service::get_bot(self.db, &bot.id).await?;
        if current.user_id != bot.user_id
            || current.platform_bot_id != bot.platform_bot_id
            || current.meta_binding.as_ref().map(|binding| {
                (
                    binding.generation,
                    &binding.grant_id,
                    &binding.token_fingerprint,
                )
            }) != bot.meta_binding.as_ref().map(|binding| {
                (
                    binding.generation,
                    &binding.grant_id,
                    &binding.token_fingerprint,
                )
            })
        {
            return Err(conflict());
        }
        let bot = &current;
        let binding = bot.meta_binding.as_ref().ok_or_else(conflict)?;
        if !bot.is_active
            || binding.provider_revoked
            || binding.grant_reconnect_required
            || binding.subscription == "reconnect_required"
            || binding
                .expires_at
                .is_none_or(|e| e > Utc::now() + Duration::days(10))
        {
            return Ok(());
        }
        scopes::require_current_operation(self.db, bot).await?;
        if binding.expires_at.is_some_and(|e| e <= Utc::now()) {
            return self.reconnect_required(bot).await;
        }
        self.refresh_inner(bot).await
    }

    async fn refresh_inner(&self, bot: &ChannelBot) -> AppResult<()> {
        let binding = bot.meta_binding.as_ref().ok_or_else(conflict)?;
        let current = super::channel_bot_service::get_bot(self.db, &bot.id).await?;
        let live = current.meta_binding.as_ref().ok_or_else(conflict)?;
        if !current.is_active
            || current.user_id != bot.user_id
            || current.platform_bot_id != bot.platform_bot_id
            || live.generation != binding.generation
            || live.token_fingerprint != binding.token_fingerprint
        {
            return Err(conflict());
        }
        // A preceding serialized refresh may already have extended the lifetime.
        if live
            .expires_at
            .is_none_or(|at| at > Utc::now() + Duration::days(10))
        {
            return Ok(());
        }
        let token = Zeroizing::new(
            super::channel_bot_service::decrypt_bot_token(self.keys, &current).await?,
        );
        let response = graph_json(
            self.http
                .get(format!(
                    "{}/refresh_access_token",
                    self.endpoints.instagram_graph
                ))
                .query(&[
                    ("grant_type", "ig_refresh_token"),
                    ("access_token", token.as_str()),
                ]),
        )
        .await;
        let value = match response {
            Ok(value) => value,
            Err(AppError::ChannelConversationNotReachable(reason))
                if reason == "meta:reconnect_required" =>
            {
                return self.reconnect_required(&current).await;
            }
            Err(error) => return Err(error),
        };
        let fresh = Zeroizing::new(
            value["access_token"]
                .as_str()
                .filter(|token| !token.is_empty())
                .ok_or_else(conflict)?
                .to_owned(),
        );
        self.adapter(&bot.platform)?
            .verify_bot_token(
                self.http,
                &BotCredentials {
                    billing: None,
                    token: &fresh,
                    platform_bot_id: Some(&bot.platform_bot_id),
                    platform_secrets: None,
                },
            )
            .await?;
        let ttl = value["expires_in"]
            .as_i64()
            .filter(|seconds| *seconds > 0 && *seconds <= 366 * 86400)
            .ok_or_else(conflict)?;
        #[cfg(test)]
        super::durable_wake_tests::after("meta_refresh_provider_confirmed").await;
        self.publish_token_result(
            &current,
            true,
            doc! {"is_active":true, "meta_binding.generation":binding.generation,
                "meta_binding.subscription":&live.subscription},
            doc! {
                "bot_token_encrypted": binary(self.keys.encrypt(fresh.as_bytes()).await?),
                "meta_binding.token_fingerprint": meta::token_fingerprint(&fresh),
                "meta_binding.expires_at": bson::DateTime::from_chrono(Utc::now() + Duration::seconds(ttl)),
                "updated_at": bson::DateTime::now(),
            },
        ).await
    }
    async fn reconnect_required(&self, bot: &ChannelBot) -> AppResult<()> {
        let binding = bot.meta_binding.as_ref().ok_or_else(conflict)?;
        let mut filter = doc! {"_id":&bot.id,"user_id":&bot.user_id,
        "platform_bot_id":&bot.platform_bot_id,"is_active":true,
        "meta_binding.generation":binding.generation,"meta_binding.token_fingerprint":&binding.token_fingerprint,
        "meta_binding.grant_reconnect_required":{"$ne":true}};
        filter.extend(grants::grant_filter(&binding.grant_id));
        self.db
            .collection::<ChannelBot>(BOTS)
            .update_one(
                filter,
                doc! {"$inc":{"meta_binding.generation":1},
                "$set":{"status": if binding.operator_suspended { "suspended" } else { "failed" },
                "webhook_registered":false,
                "meta_binding.subscription": if binding.operator_suspended { "suspended" } else { "reconnect_required" },
                "meta_binding.grant_reconnect_required": true,
                "error":"Reconnect this inbox: its grant expired or was revoked",
                "diagnostics.failure_reason":"reconnect_required",
                "updated_at":bson::DateTime::now()}},
            )
            .await?;

        Ok(())
    }
}

pub async fn sweep(state: &crate::AppState) -> AppResult<()> {
    provider::reconcile_requests(state).await?;
    sweep_service(state, &MetaService::from(state)).await
}

pub(crate) async fn claim_due(db: &mongodb::Database) -> AppResult<Option<(ChannelBot, String)>> {
    let now = bson::DateTime::now();
    let lease = uuid::Uuid::new_v4().to_string();
    let filter = doc! {
        "platform":{"$in":["messenger","instagram"]},
        "$or":[
            {"meta_binding.subscription":{"$in":["active","pending","removal_pending","suspended"]}},
            {"is_active":false,"credential_source":{"$ne":"platform"},"meta_binding.subscription":"removed"},
        ],
        "$and":[
            {"$or":[{"meta_binding.next_reconcile_at":null},{"meta_binding.next_reconcile_at":{"$lte":now}}]},
            {"$or":[{"meta_reconcile_lease_until":null},{"meta_reconcile_lease_until":{"$lte":now}}]}
        ]
    };
    let bot=db.collection::<ChannelBot>(BOTS).find_one_and_update(filter,doc! {"$set":{
        "meta_reconcile_lease_id":&lease,
        "meta_reconcile_lease_until":bson::DateTime::from_chrono(Utc::now()+Duration::seconds(120)),
        "meta_binding.next_reconcile_at":bson::DateTime::from_chrono(Utc::now()+Duration::minutes(5))
    }}).sort(doc! {"meta_binding.next_reconcile_at":1,"_id":1}).return_document(mongodb::options::ReturnDocument::After).await?;
    Ok(bot.map(|bot| (bot, lease)))
}

pub(crate) async fn sweep_service(
    state: &crate::AppState,
    service: &MetaService<'_>,
) -> AppResult<()> {
    use futures::{StreamExt, stream};
    let outcomes=stream::iter(0..16).map(|_| async {
        let Some((bot,lease))=claim_due(service.db).await? else {return Ok(());};
        let result=if !bot.is_active {
            let manual_purge = grants::manual_purge_authorized(service.db,&bot).await?;
            let result = service.disconnect(&bot).await;
            if result.is_ok() && bot.credential_source != "platform"
                && manual_purge
                && let Ok(live) = super::channel_bot_service::get_bot(service.db, &bot.id).await
                && live.meta_binding.as_ref().is_some_and(|binding| binding.subscription == "removed"
                    && bot.meta_binding.as_ref().is_some_and(|captured|captured.grant_id==binding.grant_id && captured.generation==binding.generation))
            {
                let grant_id = live.meta_binding.as_ref().map(|binding| binding.grant_id.as_str()).unwrap_or_default();
                if let Err(error) = super::personal_erasure_service::erase_provider_inbox(state, &bot.id, grant_id).await {
                    tracing::warn!(bot_id=%bot.id,error_code=error.error_code(),"Manual Meta purge remains pending");
                }
            }
            result
        } else {
            match service.refresh(&bot).await {
                Ok(())=>{
                    let live=super::channel_bot_service::get_bot(service.db,&bot.id).await?;
                    if live.meta_binding.as_ref().map(|b| b.generation) != bot.meta_binding.as_ref().map(|b| b.generation)
                        || live.meta_binding.as_ref().is_some_and(|b|b.subscription=="reconnect_required") {Ok(())}
                    else if live.meta_binding.as_ref().is_some_and(|b|b.subscription=="suspended") {
                        service.db.collection::<ChannelBot>(BOTS).update_one(
                            doc! {"_id":&live.id,"meta_binding.generation":live.meta_binding.as_ref().map(|b|b.generation)},
                            doc! {"$set":{"meta_binding.next_reconcile_at":bson::DateTime::from_chrono(Utc::now()+Duration::days(1))}},
                        ).await?;
                        Ok(())
                    } else {service.verify(&live).await}
                },
                Err(error)=>Err(error),
            }
        };
        if let Err(error)=result {
            // Provider rejection is recorded where its exact credential was
            // observed. The sweep's claimed token may have been refreshed.
            tracing::warn!(platform=%bot.platform,bot_id=%bot.id,error_code=error.error_code(),"Meta inbox lifecycle reconciliation deferred");
        }
        service.db.collection::<bson::Document>(BOTS).update_one(doc! {"_id":&bot.id,
            "meta_reconcile_lease_id":&lease},
        doc! {"$unset":{"meta_reconcile_lease_id":"",
                "meta_reconcile_lease_until":""}}).await?;

        Ok::<(),AppError>(())
    }).buffer_unordered(4).collect::<Vec<_>>().await;
    for outcome in outcomes {
        outcome?;
    }
    service
        .db
        .collection::<OAuthSession>(SESSIONS)
        .delete_many(doc! {"expires_at":{"$lte":bson::DateTime::now()}})
        .await?;
    Ok(())
}

pub async fn ensure_indexes(db: &mongodb::Database) -> mongodb::error::Result<()> {
    use mongodb::{IndexModel, options::IndexOptions};
    scopes::ensure_indexes(db).await?;
    targets::ensure_indexes(db).await?;
    scope_erasure::ensure_indexes(db).await?;
    db.collection::<bson::Document>(provider::REQUESTS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"platform":1,"app_id":1,"status":1,"_id":1})
                .build(),
        )
        .await?;
    db.collection::<OAuthSession>(SESSIONS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"grant_identity":1,"_id":1})
                .build(),
        )
        .await?;
    db.collection::<OAuthSession>(SESSIONS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"platform":1,"app_id":1,"_id":1})
                .build(),
        )
        .await?;
    db.collection::<OAuthSession>(SESSIONS)
        .create_indexes([
            IndexModel::builder()
                .keys(doc! {"state_hash":1})
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            IndexModel::builder()
                .keys(doc! {"expires_at":1})
                .options(
                    IndexOptions::builder()
                        .expire_after(std::time::Duration::ZERO)
                        .build(),
                )
                .build(),
        ])
        .await?;
    db.collection::<ChannelBot>(BOTS).create_index(IndexModel::builder()
        .keys(doc! {"platform":1,"meta_binding.app_id":1,"meta_binding.grant_subject_id":1,"_id":1}).build()).await?;
    db.collection::<OAuthSession>(SESSIONS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"platform":1,"app_id":1,"grant_subject_id":1,"created_at":1,"_id":1})
                .build(),
        )
        .await?;
    db.collection::<bson::Document>("channel_meta_erasure_fences")
        .create_index(
            IndexModel::builder()
                .keys(doc! {"retired_grant.identity":1,"_id":1})
                .build(),
        )
        .await?;
    db.collection::<ChannelBot>(BOTS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"platform":1,"meta_binding.app_id":1,"_id":1})
                .build(),
        )
        .await?;
    db.collection::<ChannelBot>(BOTS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"platform":1,
                "meta_binding.subscription":1,
                "meta_binding.next_reconcile_at":1,
                "_id":1})
                .build(),
        )
        .await?;

    db.collection::<bson::Document>(meta::CONTROLS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"identity_key":1})
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;
    db.collection::<bson::Document>("channel_meta_grant_authority")
        .create_index(
            IndexModel::builder()
                .keys(doc! {"identity":1})
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;
    db.collection::<bson::Document>("channel_meta_grant_authority")
        .create_index(
            IndexModel::builder()
                .keys(doc! {"platform":1,"app_id":1,"identity":1})
                .build(),
        )
        .await?;
    db.collection::<bson::Document>("channel_meta_grant_authority")
        .create_indexes([
            IndexModel::builder()
                .keys(doc! {"platform":1,"app_id":1,"outstanding":1,"_id":1})
                .build(),
            IndexModel::builder()
                .keys(doc! {"outstanding":1,"_id":1})
                .build(),
        ])
        .await?;
    db.collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME).create_index(
        IndexModel::builder().keys(doc! {"event_type":1,"event_data.bot_id":1,"event_data.owner_user_id":1,"created_at":-1}).build()
    ).await?;
    db.collection::<bson::Document>("channel_meta_erasure_fences")
        .create_index(
            IndexModel::builder()
                .keys(doc! {"retired_grant.identity":1,"retired_grant.authorized_at":1})
                .build(),
        )
        .await?;
    db.collection::<bson::Document>("channel_meta_erasure_fences")
        .create_index(
            IndexModel::builder()
                .keys(doc! {"retired_grant.platform":1,"retired_grant.app_id":1})
                .build(),
        )
        .await?;
    db.collection::<bson::Document>(provider::REQUESTS)
        .create_indexes([
            IndexModel::builder()
                .keys(doc! {"request_hash":1})
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            IndexModel::builder()
                .keys(doc! {"status":1,"next_attempt_at":1})
                .build(),
            IndexModel::builder()
                .keys(doc! {"expires_at":1})
                .options(
                    IndexOptions::builder()
                        .expire_after(std::time::Duration::ZERO)
                        .partial_filter_expression(doc! {"status":"completed"})
                        .build(),
                )
                .build(),
        ])
        .await?;
    Ok(())
}

#[cfg(test)]
#[path = "channel_meta_tests.rs"]
mod tests;

#[path = "channel_meta_provider.rs"]
pub(crate) mod provider;
pub(crate) use provider::fence_grant_in_session;
pub use provider::{accept_provider_request, request_status};

#[path = "channel_meta_grants.rs"]
pub(crate) mod grants;

#[path = "channel_meta_scopes.rs"]
pub(crate) mod scopes;

#[path = "channel_meta_targets.rs"]
pub(crate) mod targets;

#[path = "channel_meta_scope_erasure.rs"]
pub(crate) mod scope_erasure;
