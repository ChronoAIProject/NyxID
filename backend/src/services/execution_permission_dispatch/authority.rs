//! Snapshot and write fences for the rows selecting a credential and destination.
//! Refreshable OAuth material is deliberately excluded; replacement epochs are not.
use super::*;

pub(super) struct Snapshot(Vec<(&'static str, Document, Vec<Document>)>);

pub(super) fn key_digest(key: &crate::models::api_key::ApiKey) -> String {
    let mut value = serde_json::to_value(key).expect("key serialization");
    for field in [
        "key_hash",
        "key_prefix",
        "last_used_at",
        // Authentication itself advances this CAS version; compare authority fields.
        "state_version",
        "created_at",
        "updated_at",
        "name",
        "description",
    ] {
        value.as_object_mut().expect("key object").remove(field);
    }
    super::super::mcp_service::canonical_sha256(value)
}

fn projection(collection: &str) -> Document {
    if collection == "api_key_credentials" {
        return doc! {"_id":1,"api_key_id":1,"user_id":1,"is_active":1,"revoked_at":1,
        "expires_at":1,"machine_external_fence":1};
    }
    if collection == "users" {
        return doc! {"_id":1,"is_active":1,"user_type":1};
    }
    if collection == "user_api_keys" {
        return doc! {"_id":1,"user_id":1,"credential_epoch":1,"deleted_at":1,"provider_config_id":1};
    }
    if collection == "notification_channels" {
        return doc! {"_id":1,"user_id":1,"approval_required":1};
    }
    let mut projection = doc! {"last_used_at":0,"updated_at":0,"created_at":0,"execution_dispatch_fence":0,
    "execution_task_fence":0,"mutation_fence":0,"reference_fence":0};
    if collection == "org_memberships" {
        // Live collaboration admission writes this lock counter in the same
        // transaction. It is not a membership/role revision; those authority
        // fields remain compared and the row remains transactionally fenced.
        projection.insert("collaboration_fence", 0);
    }
    // Keep the required shape for typed normalization of legacy defaults. The
    // snapshot stays private and never implements Debug or leaves the process.
    if collection == "api_keys" {
        projection.remove("created_at");
        // validate_api_key records usage through the mutation helper, which also
        // increments state_version. Actual authorization fields remain fenced.
        projection.insert("state_version", 0);
    }
    projection
}

fn require_live_credential(rows: &[Document]) -> AppResult<()> {
    let row = rows
        .first()
        .filter(|_| rows.len() == 1)
        .ok_or_else(refused)?;
    let unexpired = match row.get("expires_at") {
        None | Some(bson::Bson::Null) => true,
        Some(bson::Bson::DateTime(end)) => end.to_chrono() > Utc::now(),
        _ => false,
    };
    if row.get_bool("is_active").ok() != Some(true)
        || !matches!(row.get("revoked_at"), None | Some(bson::Bson::Null))
        || !unexpired
    {
        return Err(refused());
    }
    Ok(())
}

impl Snapshot {
    pub(super) fn require_key(&self, expected: &str) -> AppResult<()> {
        let row = self
            .0
            .iter()
            .find(|(name, _, _)| *name == "api_keys")
            .and_then(|(_, _, rows)| rows.first())
            .ok_or_else(refused)?;
        let key = bson::from_document(row.clone()).map_err(encoding)?;
        if key_digest(&key) != expected {
            return Err(refused());
        }
        Ok(())
    }
    pub(super) async fn load(
        db: &Database,
        connection: &str,
        key: &str,
        credential: Option<&str>,
    ) -> AppResult<Self> {
        let services = db
            .collection::<Document>("user_services")
            .find(doc! {"_id":connection})
            .projection(projection("user_services"))
            .await?
            .try_collect::<Vec<_>>()
            .await?;
        let service = services.first().ok_or_else(refused)?;
        let endpoint = service.get_str("endpoint_id").map_err(|_| refused())?;
        let catalog = service.get_str("catalog_service_id").ok();
        let bindings = db
            .collection::<Document>("agent_service_bindings")
            .find(doc! {"api_key_id":key,"user_service_id":connection})
            .projection(projection("agent_service_bindings"))
            .sort(doc! {"_id":1})
            .await?
            .try_collect::<Vec<_>>()
            .await?;
        let mut credentials = Vec::new();
        if let Ok(id) = service.get_str("api_key_id") {
            credentials.push(id.to_owned());
        }
        for binding in &bindings {
            if let Ok(id) = binding.get_str("user_api_key_id") {
                credentials.push(id.to_owned());
            }
        }
        let specs = [
            ("api_keys", doc! {"_id":key}),
            ("user_endpoints", doc! {"_id":endpoint}),
            (
                "downstream_services",
                doc! {"_id":{"$in":catalog.into_iter().collect::<Vec<_>>()}},
            ),
            ("user_api_keys", doc! {"_id":{"$in":credentials}}),
        ];
        let mut rows = Vec::new();
        for (collection, filter) in specs {
            rows.push((
                collection,
                filter.clone(),
                db.collection::<Document>(collection)
                    .find(filter)
                    .projection(projection(collection))
                    .sort(doc! {"_id":1})
                    .await?
                    .try_collect::<Vec<_>>()
                    .await?,
            ));
        }
        let actor = rows
            .iter()
            .find(|(name, _, _)| *name == "api_keys")
            .and_then(|(_, _, rows)| rows.first())
            .and_then(|key| key.get_str("user_id").ok())
            .ok_or_else(refused)?
            .to_owned();
        if let Some(credential) = credential {
            let filter = doc! {"_id":credential,"api_key_id":key,"user_id":&actor};
            let current = db
                .collection::<Document>("api_key_credentials")
                .find(filter.clone())
                .projection(projection("api_key_credentials"))
                .await?
                .try_collect::<Vec<_>>()
                .await?;
            require_live_credential(&current)?;
            rows.push(("api_key_credentials", filter, current));
        }
        let memberships = db
            .collection::<Document>("org_memberships")
            .find(doc! {"member_user_id":&actor})
            .projection(projection("org_memberships"))
            .sort(doc! {"_id":1})
            .limit(257)
            .await?
            .try_collect::<Vec<_>>()
            .await?;
        if memberships.len() > 256 {
            return Err(refused());
        }
        rows.push((
            "org_memberships",
            doc! {"member_user_id":&actor},
            memberships,
        ));
        let owner = service.get_str("user_id").map_err(|_| refused())?;
        for (collection, filter) in [
            ("users", doc! {"_id":{"$in":[&actor,owner]}}),
            (
                "service_approval_configs",
                doc! {"user_id":{"$in":[&actor,owner]},"service_id":{"$in":[connection,catalog.unwrap_or(connection)]}},
            ),
            (
                "notification_channels",
                doc! {"user_id":{"$in":[&actor,owner]}},
            ),
        ] {
            let current = db
                .collection::<Document>(collection)
                .find(filter.clone())
                .projection(projection(collection))
                .sort(doc! {"_id":1})
                .await?
                .try_collect()
                .await?;
            rows.push((collection, filter, current));
        }
        rows.push(("user_services", doc! {"_id":connection}, services));
        rows.push((
            "agent_service_bindings",
            doc! {"api_key_id":key,"user_service_id":connection},
            bindings,
        ));
        Ok(Self(rows))
    }

    pub(super) async fn fence(&self, db: &Database, session: &mut ClientSession) -> AppResult<()> {
        for (collection, filter, expected) in &self.0 {
            let rows = db
                .collection::<Document>(collection)
                .find(filter.clone())
                .projection(projection(collection))
                .sort(doc! {"_id":1})
                .session(&mut *session)
                .await?
                .stream(&mut *session)
                .try_collect::<Vec<_>>()
                .await?;
            if *collection == "api_key_credentials" {
                require_live_credential(&rows)?;
            }
            if &rows != expected {
                return Err(refused());
            }
            if db
                .collection::<Document>(collection)
                .update_many(
                    filter.clone(),
                    doc! {"$inc":{"execution_dispatch_fence":1_i64}},
                )
                .session(&mut *session)
                .await?
                .matched_count
                != expected.len() as u64
            {
                return Err(refused());
            }
        }
        Ok(())
    }

    pub(super) fn digest(&self) -> String {
        super::super::mcp_service::canonical_sha256(
            serde_json::to_value(&self.0).expect("snapshot encoding"),
        )
    }

    pub(super) fn same(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
