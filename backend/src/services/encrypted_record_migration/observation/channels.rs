use super::{companions::Parents, *};
use crate::crypto::token::hash_token;
use crate::models::{
    channel_connect_link::{ChannelConnectLink, Receiver},
    channel_message::ChannelMessage,
    channel_meta::OAuthSession,
};
use crate::services::{
    channel_bot_record as bot, channel_connect_link_service as links, channel_guest_access::relay,
    channel_meta_service::asset_record as assets, nyxbot_channel_key,
    nyxbot_event_binding_service::storage as event_binding, record_field_service::stored_bytes,
    whatsapp_native::token_record,
};
use base64::{Engine, engine::general_purpose::STANDARD};

pub(super) fn source_companions(entry: &Entry) -> Option<&'static [&'static str]> {
    Some(match entry.adapter {
        Adapter::ChannelBot => &[
            "user_id",
            "ownership_version",
            "credential_continuity",
            "credential_source",
            "is_active",
            "platform",
            "meta_binding",
            "webhook_secret_hash",
            "secret_context_versions",
        ],
        Adapter::ChannelReceiver => &["user_id", "receiver", "secret_context_versions"],
        Adapter::NyxbotChannelKey => &[
            "user_id",
            "agent_api_key_id",
            "status",
            "transport",
            "secret_context_versions",
        ],
        Adapter::EventBinding => &[
            "user_id",
            "agent_id",
            "agent_owner_id",
            "provider_api_key_id",
            "producer_key_id",
            "secret_context_versions",
            "status",
            "revoked_at",
        ],
        Adapter::MetaConsentAssets => &[
            "owner_id",
            "actor_id",
            "platform",
            "app_id",
            "state_hash",
            "created_at",
            "bot_id",
            "reconnect_bot_id",
            "expected_generation",
            "grant_subject_id",
            "grant_identity",
            "selected_asset_id",
            "status",
            "assets_context_version",
        ],
        Adapter::NativeWhatsapp => &["owner_id", "ownership_version", "secret_context_versions"],
        Adapter::ChannelGuestReplyHandle => &[
            "user_id",
            "guest_relay",
            "platform",
            "direction",
            "channel_bot_id",
            "platform_message_id",
            "conversation_id",
            "platform_conversation_id",
        ],
        _ => return None,
    })
}

pub(super) fn marked(v: Option<u8>) -> Marker {
    v.map_or(Marker::Unmarked, |v| Marker::Version(v.into()))
}

impl Source<'_> {
    async fn bot(&self, out: &mut Observation) -> AppResult<()> {
        let decoded = bot::decode(self.row);
        out.check(
            "bot_token_encrypted",
            decoded.as_ref().map(|_| ()).map_err(|_| invalid()),
        );
        let versions = self.versions()?;
        let owner = text(self.row, "user_id")?;
        for &field in self.family.entry.fields {
            let result = async {
                let marker = version(versions.and_then(|v| v.get(field)))?;
                let Some(raw) = present(self.row, field) else {
                    if marker.is_some() || field == "bot_token_encrypted" {
                        return Err(invalid());
                    }
                    out.instances.push((field, Storage::Absent));
                    return Ok(());
                };
                if decoded
                    .as_ref()
                    .is_ok_and(|b| bot::external_token(b, field))
                {
                    out.instances.push((field, Storage::Sentinel));
                    return Ok(());
                }
                self.record(
                    out,
                    field,
                    owner,
                    &[field],
                    marker.map_or(Marker::Unmarked, Marker::Version),
                    &stored_bytes(raw)?,
                    |bytes| {
                        let s = std::str::from_utf8(bytes).map_err(|_| invalid())?;
                        if field == "webhook_secret_encrypted"
                            && self.row.get_str("webhook_secret_hash") != Ok(hash_token(s).as_str())
                        {
                            return Err(invalid());
                        }
                        Ok(())
                    },
                )
                .await
            }
            .await;
            out.check(field, result);
        }
        Ok(())
    }

    async fn binding_keys(&self, out: &mut Observation, parents: &Parents) -> AppResult<()> {
        let channel = matches!(self.family.entry.adapter, Adapter::NyxbotChannelKey);
        let checked = if channel {
            nyxbot_channel_key::snapshot(self.row).map(drop)
        } else {
            event_binding::snapshot(self.row).map(drop)
        };
        out.check(self.family.entry.fields[0], checked);
        let versions = self.versions()?;
        for &field in self.family.entry.fields {
            let result = async {
                let Some(raw) = present(self.row, field) else {
                    out.instances.push((field, Storage::Absent));
                    return Ok(());
                };
                let owner = text(self.row, "user_id")?;
                let producer = field == "producer_secret_ciphertext";
                let mut path = vec![
                    field,
                    text(
                        self.row,
                        if channel {
                            "agent_api_key_id"
                        } else if producer {
                            "producer_key_id"
                        } else {
                            "provider_api_key_id"
                        },
                    )?,
                ];
                if !channel {
                    path.extend([
                        text(self.row, "agent_id")?,
                        text(self.row, "agent_owner_id")?,
                    ]);
                }
                let marker = version(versions.and_then(|v| v.get(field)))?
                    .map_or(Marker::Unmarked, Marker::Version);
                let parent = parents.get("api_keys");
                if !producer && parent.is_none() {
                    out.findings.push((field, Finding::MissingParent));
                }
                self.record(out, field, owner, &path, marker, &stored_bytes(raw)?, |p| {
                    if !producer {
                        return super::retained::issued_key(parent, owner, p);
                    }
                    let mut decoded = zeroize::Zeroizing::new([0u8; 32]);
                    (p.len() == 44 && STANDARD.decode_slice(p, decoded.as_mut()).ok() == Some(32))
                        .then_some(())
                        .ok_or_else(invalid)
                })
                .await
            }
            .await;
            out.check(field, result);
        }
        Ok(())
    }

    async fn meta(&self, out: &mut Observation) -> AppResult<()> {
        let f = self.family.entry.fields[0];
        let row: OAuthSession = bson::from_document(self.row.clone()).map_err(|_| invalid())?;
        let Some(bytes) = &row.assets_encrypted else {
            if row.assets_context_version.is_some()
                || !matches!(
                    row.status.as_str(),
                    "pending"
                        | "exchanging"
                        | "no_eligible_assets"
                        | "completed"
                        | "failed"
                        | "cancelled"
                        | "revoked"
                )
            {
                return Err(invalid());
            }
            out.instances.push((f, Storage::Absent));
            return Ok(());
        };
        if !matches!(
            (row.status.as_str(), row.selected_asset_id.is_some()),
            ("assets", false) | ("installing", true)
        ) {
            out.findings.push((f, Finding::AmbiguousBinding));
        }
        let subject = row.grant_subject_id.as_deref();
        if subject.is_none() {
            out.findings.push((f, Finding::AmbiguousBinding));
            if row.assets_context_version.is_some() {
                return Ok(());
            }
        }
        // Unmarked envelopes have no AAD. A missing subject still permits key
        // attribution, but the consent attempt cannot be authenticated.
        let subject = subject.unwrap_or_default();
        let attempt = assets::attempt(&row)?;
        let path = assets::path(&row, subject, &attempt);
        // A contextual identity cannot have an empty segment.
        let legacy_path = [f];
        let path = if row.assets_context_version.is_none() {
            &legacy_path[..]
        } else {
            &path[..]
        };
        self.record(
            out,
            f,
            &row.owner_id,
            path,
            marked(row.assets_context_version),
            bytes,
            |p| assets::decode(&row, subject, p).map(drop),
        )
        .await?;
        if out.findings.contains(&(f, Finding::InvalidValue)) {
            out.findings.push((f, Finding::AmbiguousBinding));
        }
        Ok(())
    }

    pub(super) async fn channel(&self, out: &mut Observation, parents: &Parents) -> AppResult<()> {
        let f = self.family.entry.fields[0];
        match self.family.entry.adapter {
            Adapter::ChannelBot => self.bot(out).await,
            Adapter::NyxbotChannelKey | Adapter::EventBinding => {
                self.binding_keys(out, parents).await
            }
            Adapter::MetaConsentAssets => self.meta(out).await,
            Adapter::ChannelReceiver => {
                let row: ChannelConnectLink =
                    bson::from_document(self.row.clone()).map_err(|_| invalid())?;
                links::validate_receiver(&row)?;
                if let Some(Receiver::Direct {
                    secret_encrypted, ..
                }) = &row.receiver
                {
                    self.record(
                        out,
                        f,
                        &row.user_id,
                        &[f],
                        marked(row.secret_context_versions.get(f).copied()),
                        secret_encrypted,
                        |p| bounded_text(p, 4096),
                    )
                    .await?;
                } else {
                    out.instances.push((f, Storage::Absent));
                }
                Ok(())
            }
            Adapter::NativeWhatsapp => {
                let row: crate::models::channel_whatsapp_native::NativeWhatsapp =
                    bson::from_document(self.row.clone()).map_err(|_| invalid())?;
                let marker = marked(token_record::version(&row)?);
                if let Some(bytes) = &row.native_verify_token_encrypted {
                    self.record(
                        out,
                        f,
                        &row.owner_id,
                        &[f, &row.ownership_version.to_string()],
                        marker,
                        bytes,
                        |p| {
                            ((16..=256).contains(&p.len())
                                && p.iter().all(|b| (33..=126).contains(b)))
                            .then_some(())
                            .ok_or_else(invalid)
                        },
                    )
                    .await?;
                } else {
                    out.instances.push((f, Storage::Absent));
                }
                Ok(())
            }
            Adapter::ChannelGuestReplyHandle => {
                let Some(thread) = present(self.row, f) else {
                    out.instances.push((f, Storage::Absent));
                    return Ok(());
                };
                let thread = thread.as_str().ok_or_else(invalid)?;
                if !thread.starts_with(relay::REPLY_HANDLE_PREFIX) {
                    if thread.starts_with("interaction:")
                        && self.row.get_str("platform") == Ok("discord")
                        && present(self.row, "guest_relay").is_some()
                    {
                        out.instances.push((f, Storage::Plaintext));
                        out.findings.push((f, Finding::InvalidValue));
                    } else {
                        out.instances.push((f, Storage::Absent));
                    }
                    return Ok(());
                }
                let row: ChannelMessage =
                    bson::from_document(self.row.clone()).map_err(|_| invalid())?;
                self.record(
                    out,
                    f,
                    &row.user_id,
                    &relay::reply_handle_fields(&row)?,
                    Marker::GuestReplyPrefixV3,
                    &relay::decode_reply_handle(&row)?,
                    |p| {
                        bounded_text(p, 2048)?;
                        p.starts_with(b"interaction:")
                            .then_some(())
                            .ok_or_else(invalid)
                    },
                )
                .await
            }
            _ => Err(invalid()),
        }
    }
}
