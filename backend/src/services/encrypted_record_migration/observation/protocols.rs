use super::{channels::marked, companions::Parents, *};
use crate::models::{
    nyxbot_gateway_response::GatewayResponse,
    platform_native_reply_handle::NativeReplyHandle,
    trigger::{Trigger, TriggerDelivery, TriggerVerification},
    trigger_delivery::TriggerDeliveryRecord,
};
use crate::services::{
    mobile_social_service::record as mobile, nyxbot_event_record as event,
    nyxbot_response_recovery::storage as replay, nyxbot_thread_reference as thread,
    oracle_login_record_service as oracle, platform_entry_gateway_service as gateway,
    platform_native_reply_service::record as native, platform_setup_record_service as setup,
    record_field_service::stored_bytes, trigger_record_service as trigger,
};

pub(super) fn source_companions(entry: &Entry) -> Option<&'static [&'static str]> {
    Some(match entry.adapter {
        Adapter::OracleLoginOuter => &[
            "pool_id",
            "generation",
            "revision",
            "format_version",
            "envelope_size",
            "expires_at",
        ],
        Adapter::NativeReplyHandle => &[
            "receipt_id",
            "binding_digest",
            "created_at",
            "expires_at",
            "ciphertext_context_version",
        ],
        Adapter::MobileSocialDelivery => &[
            "user_id",
            "code_hash",
            "code_challenge",
            "ciphertext_context_version",
        ],
        Adapter::Setup => &[
            "owner_user_id",
            "idempotency_scope",
            "template",
            "state",
            "owner_retirement",
            "published",
            "facts_context_version",
        ],
        Adapter::GatewayTransport => &[
            "credential_owner_id",
            "request_hash",
            "secret_context_versions",
        ],
        Adapter::GatewayResponse => &[
            "user_id",
            "channel_id",
            "binding_id",
            "response_id",
            "partition",
            "terminal",
            "payload_context_version",
        ],
        Adapter::ThreadReference => &["user_id", "channel_id", "event_ref_context_version"],
        Adapter::NyxbotEvent => &[
            "user_id",
            "channel_id",
            "event_id",
            "custom_binding_id",
            "partition",
            "guest_setup_conversation_id",
            "delivery",
            "event_context_version",
            "guest_setup_context_version",
            "channel_guest_setup_context_version",
            "response_reply",
            "response_read",
            "org_notice",
        ],
        Adapter::Trigger if entry.collection == "triggers" => &[
            "user_id",
            "verification",
            "delivery",
            "secret_hash",
            "secret_context_versions",
        ],
        Adapter::Trigger => &[
            "user_id",
            "trigger_id",
            "event_id",
            "secret_context_versions",
        ],
        _ => return None,
    })
}

impl Source<'_> {
    async fn event(&self, out: &mut Observation) -> AppResult<()> {
        let populated = |field| event::stored(self.row, field).map(|v| v.is_some());
        if (populated(event::GUEST)? || populated(event::GUEST_VERSION)?)
            && (populated(event::CHANNEL_GUEST)? || populated(event::CHANNEL_GUEST_VERSION)?)
        {
            out.findings.push((event::GUEST, Finding::Malformed));
        }
        for &(field, marker) in event::FIELDS {
            let result = async {
                let v = version(event::stored(self.row, marker)?)?;
                let Some(raw) = event::stored(self.row, field)? else {
                    if v.is_some()
                        || (field == event::REPLY && present(self.row, "response_reply").is_some())
                    {
                        return Err(invalid());
                    }
                    out.instances.push((field, Storage::Absent));
                    return Ok(());
                };
                let mut path = vec![field, text(self.row, "channel_id")?];
                if field == event::REPLY {
                    path.push(
                        event::stored(self.row, "response_reply.response_id")?
                            .and_then(Bson::as_str)
                            .ok_or_else(invalid)?,
                    );
                }
                self.record(
                    out,
                    field,
                    text(self.row, "user_id")?,
                    &path,
                    v.map_or(Marker::Unmarked, Marker::Version),
                    &stored_bytes(raw)?,
                    |p| event::validate_plain(self.row, field, p),
                )
                .await
            }
            .await;
            out.check(field, result);
        }
        Ok(())
    }

    async fn triggers(&self, out: &mut Observation) -> AppResult<()> {
        let fields = self.family.entry.fields;
        out.check(
            fields[0],
            trigger::validate_versions(self.row, fields).map(drop),
        );
        let versions = self.versions()?;
        let marker = |f| {
            version(versions.and_then(|v| v.get(f)))
                .map(|v| v.map_or(Marker::Unmarked, Marker::Version))
        };
        if self.family.collection() == "trigger_deliveries" {
            let row: TriggerDeliveryRecord =
                bson::from_document(self.row.clone()).map_err(|_| invalid())?;
            let f = fields[0];
            if let Some(bytes) = &row.envelope_encrypted {
                self.record(
                    out,
                    f,
                    &row.user_id,
                    &[f, "trigger_id", &row.trigger_id, "event_id", &row.event_id],
                    marker(f)?,
                    bytes,
                    |p| trigger::validate_envelope(&row, p),
                )
                .await?;
            } else {
                out.instances.push((f, Storage::Absent));
            }
            return Ok(());
        }
        let mut metadata = self.row.clone();
        for &field in fields {
            metadata.remove(field);
        }
        let row: Trigger = bson::from_document(metadata).map_err(|_| invalid())?;
        for &field in fields {
            let result = async {
                let Some(raw) = present(self.row, field) else {
                    if (field == fields[0]
                        && matches!(row.verification, TriggerVerification::HmacSha256 { .. }))
                        || (field == fields[1]
                            && matches!(row.delivery, TriggerDelivery::Webhook { .. }))
                    {
                        return Err(invalid());
                    }
                    out.instances.push((field, Storage::Absent));
                    return Ok(());
                };
                self.record(
                    out,
                    field,
                    &row.user_id,
                    &[field],
                    marker(field)?,
                    &stored_bytes(raw)?,
                    |p| {
                        if p.is_empty() || p.len() > 8192 {
                            return Err(invalid());
                        }
                        if field == fields[0]
                            && crate::crypto::token::hash_token(
                                std::str::from_utf8(p).map_err(|_| invalid())?,
                            ) != row.secret_hash
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

    pub(super) async fn protocol(&self, out: &mut Observation, parents: &Parents) -> AppResult<()> {
        let f = self.family.entry.fields[0];
        match self.family.entry.adapter {
            Adapter::OracleLoginOuter => {
                let outer = oracle::outer_at(self.family.collection(), self.row, self.as_of)?;
                if outer.ciphertext.is_empty() {
                    out.instances.push((f, Storage::Sentinel));
                    return Ok(());
                }
                out.open(
                    self.keys,
                    f,
                    Marker::Unmarked,
                    Binding::Oracle(&outer.binding),
                    &outer.ciphertext,
                    |p| {
                        (p.len() as u64 == outer.size)
                            .then_some(())
                            .ok_or_else(invalid)
                    },
                )
                .await;
                Ok(())
            }
            Adapter::NativeReplyHandle => {
                let row: NativeReplyHandle =
                    bson::from_document(self.row.clone()).map_err(|_| invalid())?;
                let Some(parent) =
                    parents.get(crate::models::platform_entry_identity::NATIVE_RECEIPTS)
                else {
                    out.findings.push((f, Finding::MissingParent));
                    return Ok(());
                };
                let receipt = bson::from_document(parent.clone()).map_err(|_| invalid())?;
                let binding = native::Binding::stored_at(&row, &receipt, self.as_of)?;
                if row.ciphertext_context_version.is_none() {
                    out.open(
                        self.keys,
                        f,
                        Marker::Unmarked,
                        Binding::Historical(&binding.historical()?),
                        &row.ciphertext,
                        |p| binding.validate_plain(p),
                    )
                    .await;
                    Ok(())
                } else {
                    self.record(
                        out,
                        f,
                        "platform",
                        &binding.path(),
                        marked(row.ciphertext_context_version),
                        &row.ciphertext,
                        |p| binding.validate_plain(p),
                    )
                    .await
                }
            }
            Adapter::MobileSocialDelivery => {
                let row = mobile::row(self.row)?;
                let binding = mobile::Binding::new(
                    &row.id,
                    &row.user_id,
                    &row.code_hash,
                    &row.code_challenge,
                );
                let context = binding.context()?;
                let legacy = binding.legacy_aad();
                let auth = match row.ciphertext_context_version {
                    None => Binding::Historical(&legacy),
                    Some(v) => Binding::Record(Some(v.into()), &context),
                };
                out.open(
                    self.keys,
                    f,
                    marked(row.ciphertext_context_version),
                    auth,
                    &row.ciphertext.bytes,
                    |p| mobile::validate_plain(p, &row.user_id),
                )
                .await;
                Ok(())
            }
            Adapter::Setup => {
                let row = setup::row(self.row)?;
                if row.facts_ciphertext.is_empty() {
                    out.instances.push((f, Storage::Sentinel));
                    return Ok(());
                }
                let owner = setup::identity(
                    &row.id,
                    row.owner_user_id.as_deref(),
                    &row.idempotency_scope,
                )?;
                self.record(
                    out,
                    f,
                    owner,
                    &[f, &row.idempotency_scope],
                    marked(row.facts_context_version),
                    &row.facts_ciphertext,
                    |p| setup::validate_plain(&row, p),
                )
                .await
            }
            Adapter::GatewayTransport => {
                let row = gateway::stored_transport(self.row)?;
                self.record(
                    out,
                    f,
                    &row.credential_owner_id,
                    &[f],
                    marked(row.secret_context_versions.get(f).copied()),
                    &row.request_ciphertext,
                    |p| {
                        (gateway::sha256(p) == row.request_hash)
                            .then_some(())
                            .ok_or_else(invalid)
                    },
                )
                .await
            }
            Adapter::GatewayResponse => {
                let row: GatewayResponse =
                    bson::from_document(self.row.clone()).map_err(|_| invalid())?;
                self.record(
                    out,
                    f,
                    &row.user_id,
                    &replay::field(&row),
                    marked(row.payload_context_version),
                    &row.payload_ciphertext,
                    |p| replay::checked(&row, p),
                )
                .await
            }
            Adapter::ThreadReference => {
                let Some(row) = thread::material(self.row)? else {
                    out.instances.push((f, Storage::Absent));
                    return Ok(());
                };
                self.record(
                    out,
                    f,
                    &row.user_id,
                    &[f, &row.channel_id],
                    marked(row.event_ref_context_version),
                    row.event_ref_ciphertext.as_deref().ok_or_else(invalid)?,
                    |p| std::str::from_utf8(p).map(drop).map_err(|_| invalid()),
                )
                .await
            }
            Adapter::NyxbotEvent => self.event(out).await,
            Adapter::Trigger => self.triggers(out).await,
            _ => Err(invalid()),
        }
    }
}
