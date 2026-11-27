//! Each identity join supplies both the forward lookup and the reverse query
//! path. Keep the requested parent identity even when the snapshot has no row.
use super::*;
use crate::services::encrypted_record_census::{Row, replay::Snapshot};
use std::collections::BTreeMap;

pub(super) type Parents = BTreeMap<&'static str, Document>;

impl Family {
    pub(crate) fn parent_joins(&self) -> &'static [(&'static str, &'static str)] {
        match self.entry.adapter {
            Adapter::AssistantCredential => &[("api_keys", "api_key_id")],
            Adapter::Attachment => &[("assistant_attachments", "parent_attachment_id")],
            Adapter::Public if self.collection() == "platform_public_sessions" => {
                &[("api_keys", "credential_api_key_id")]
            }
            Adapter::Public => &[("platform_public_sessions", "public_session_id")],
            _ => &[],
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
            if let Ok(id) = source.get_str(path) {
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
        let mut out = self.observe(keys, source, &parents).await;
        out.parents = edges;
        Ok(out)
    }
}
