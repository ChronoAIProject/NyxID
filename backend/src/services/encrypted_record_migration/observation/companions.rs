//! Each identity join supplies both the forward lookup and the reverse query
//! path. Keep the requested parent identity even when the snapshot has no row.
use super::*;
use crate::services::encrypted_record_census::{Row, replay::Snapshot};
use bson::doc;
use futures::TryStreamExt;
use std::collections::BTreeMap;

pub(super) type Parents = BTreeMap<&'static str, Document>;

impl Family {
    pub(crate) fn parent_joins(&self) -> &'static [(&'static str, &'static str)] {
        match self.entry.adapter {
            Adapter::AssistantCredential => &[("api_keys", "api_key_id")],
            Adapter::NyxbotChannelKey => &[("api_keys", "agent_api_key_id")],
            Adapter::EventBinding => &[("api_keys", "provider_api_key_id")],
            Adapter::NativeReplyHandle => &[("platform_native_entry_receipts", "receipt_id")],
            Adapter::Attachment => &[("assistant_attachments", "parent_attachment_id")],
            Adapter::Public if self.collection() == "platform_public_sessions" => {
                &[("api_keys", "credential_api_key_id")]
            }
            Adapter::Public => &[("platform_public_sessions", "public_session_id")],
            Adapter::NodeDevice => &[
                ("api_keys", "issued_api_key_id"),
                ("nodes", "issued_node_id"),
            ],
            Adapter::AuthDevice => &[("api_key_credentials", "agent_key_grant.credential_id")],
            _ => &[],
        }
    }

    /// Historical refresh JWTs name a jti rather than the Mongo _id. On a
    /// refresh-token change the reverse join must rescan, including prior misses.
    pub(crate) fn rescan_parents(&self) -> &'static [&'static str] {
        if matches!(self.entry.adapter, Adapter::AuthDevice) {
            &["refresh_tokens"]
        } else {
            &[]
        }
    }

    pub(crate) async fn observe_at(
        &self,
        keys: &EncryptionKeys,
        source: &Document,
        at: &Snapshot<'_>,
    ) -> AppResult<Observation> {
        let mut parents = Parents::new();
        let mut edges = Vec::new();
        for &(collection, path) in self.parent_joins() {
            if matches!(
                self.entry.adapter,
                Adapter::AuthDevice | Adapter::NodeDevice
            ) && present(source, "delivery_context_version").is_some()
            {
                continue;
            }
            let id = match path.split_once('.') {
                Some((outer, inner)) => source
                    .get_document(outer)
                    .and_then(|doc| doc.get_str(inner)),
                None => source.get_str(path),
            };
            if let Ok(id) = id {
                let row = Row {
                    collection: collection.into(),
                    id: id.into(),
                };
                if let Some(parent) = at.row(&row).await? {
                    parents.insert(collection, parent);
                }
                edges.push(row);
            }
        }
        // A cut has second precision. A deadline in the same second stays
        // unresolved until a later cut can prove that it passed.
        let as_of =
            chrono::DateTime::from_timestamp(i64::from(at.cut.time.time), 0).ok_or_else(invalid)?;
        let mut out = self.observe_at_time(keys, source, &parents, as_of).await;
        if let Some(jti) = out.refresh_jti.take() {
            let &[collection] = self.rescan_parents() else {
                return Err(invalid());
            };
            let mut rows = at
                .find(
                    collection,
                    doc! {"jti":jti.as_str()},
                    doc! {"user_id":1,"session_id":1,"jti":1},
                )
                .await?;
            let field = "delivery_refresh_token_encrypted";
            match rows.try_next().await? {
                None => out.findings.push((field, Finding::MissingParent)),
                Some(row) => {
                    let valid = row.get_str("jti") == Ok(jti.as_str())
                        && row.get_str("user_id").ok() == source.get_str("approved_user_id").ok()
                        && row.get_str("session_id").ok()
                            == source.get_str("approved_session_id").ok()
                        && rows.try_next().await?.is_none();
                    if !valid {
                        out.findings.push((field, Finding::InvalidValue));
                    }
                }
            }
        }
        out.parents = edges;
        Ok(out)
    }
}
