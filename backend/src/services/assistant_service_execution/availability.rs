use super::*;

/// Accumulates classified failures for the complete permitted route set. One
/// policy/signature failure disqualifies the set; one successful route never
/// reaches the exhausted-routes recorder.
#[derive(Default)]
pub struct NodeFailureEvidence {
    attempted: bool,
    disqualified: bool,
    dispatched: bool,
}

impl NodeFailureEvidence {
    pub fn record(&mut self, error: &AppError, dispatched: bool) {
        self.attempted = true;
        self.dispatched |= dispatched;
        self.disqualified |= !matches!(
            error,
            AppError::NodeOffline(_)
                | AppError::NodeCredentialMissing(_)
                | AppError::NodeProxyTimeout
        );
    }

    pub fn disqualify(&mut self) {
        self.disqualified = true;
    }
}

pub(super) async fn verified_unavailable(
    db: &Database,
    nodes: &crate::services::node_ws_manager::NodeWsManager,
    chat: &acks::ChatAuthority,
    service: &UserService,
    operation: &Value,
) -> AppResult<bool> {
    if let Some(id) = service.node_id.as_deref() {
        let key =
            crate::services::key_service::get_api_key(db, &chat.user_id, &chat.api_key_id).await?;
        let access = crate::services::org_agent_service::resolve_key_access(
            db,
            &key.user_id,
            key.assistant_agent_owner_id.as_deref(),
        )
        .await?;
        let (all, allowed) =
            crate::services::org_agent_service::key_nodes(db, &key, access.as_deref()).await?;
        let owner =
            crate::services::org_service::resolve_owner_access(db, &chat.user_id, &service.user_id)
                .await?;
        let mut routes = crate::services::node_routing_service::list_dispatchable_binding_node_ids(
            db,
            &service.user_id,
            service.catalog_service_id.as_deref().unwrap_or(&service.id),
            nodes,
        )
        .await?;
        if crate::services::node_routing_service::is_node_id_dispatchable(db, id, nodes).await? {
            routes.push(id.to_owned());
        }
        if !routes
            .iter()
            .any(|id| (all || allowed.contains(id)) && owner.allows_resource(id))
        {
            return Ok(true);
        }
    }
    if let Some(id) = &service.api_key_id
        && let Some(key) = db
            .collection::<UserApiKey>("user_api_keys")
            .find_one(doc! {"_id": id, "user_id": &service.user_id})
            .await?
    {
        // Expiry alone is not proof: refresh may recover it.
        if matches!(key.status.as_str(), "revoked" | "failed" | "refresh_failed") {
            return Ok(true);
        }
    }
    if !chat.turn_live || chat.turn_stopped {
        return Ok(false);
    }
    let mut filter = scope(chat)?;
    filter.extend(
        doc! {"kind": "unavailable_bucket", "entries": {"$elemMatch": {
            "connection_id": &service.id, "operation": digest(operation.clone()),
            "binding": connection_binding(db, &service.id, &chat.api_key_id).await?,
            "expires_at": {"$gt":bson::DateTime::now()},
        }}},
    );
    Ok(db
        .collection::<Document>(COLLECTION)
        .find_one(filter)
        .await?
        .is_some())
}

/// Record only classified transport evidence attached to the private admission.
/// A credential/config replacement cannot inherit an earlier outage exemption.
impl Admission {
    pub fn exhausted_node_routes<'a>(
        &'a self,
        db: &'a Database,
        chat: &'a acks::ChatAuthority,
        failures: &'a NodeFailureEvidence,
    ) -> futures::future::BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            if failures.attempted && !failures.disqualified && (!failures.dispatched || self.read) {
                self.record_unavailable(
                    db,
                    chat,
                    if failures.dispatched {
                        "read_temporary"
                    } else {
                        "node_pre_dispatch"
                    },
                )
                .await?;
            }
            Ok(())
        })
    }

    pub fn transport_failure<'a>(
        &'a self,
        db: &'a Database,
        chat: &'a acks::ChatAuthority,
        error: &'a crate::services::proxy_service::ForwardRequestError,
    ) -> futures::future::BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            if let crate::services::proxy_service::ForwardRequestError::Transport(error) = error {
                if error.is_connect() {
                    self.record_unavailable(db, chat, "pre_dispatch").await?;
                } else if self.read && error.is_timeout() {
                    self.record_unavailable(db, chat, "read_temporary").await?;
                }
            }
            Ok(())
        })
    }

    pub fn response<'a>(
        &'a self,
        db: &'a Database,
        chat: &'a acks::ChatAuthority,
        status: u16,
    ) -> futures::future::BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            if self.read && matches!(status, 429 | 502 | 503 | 504) {
                self.record_unavailable(db, chat, "read_temporary").await?;
            }
            Ok(())
        })
    }

    pub(crate) fn record_unavailable<'a>(
        &'a self,
        db: &'a Database,
        chat: &'a acks::ChatAuthority,
        evidence: &'a str,
    ) -> futures::future::BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            chat.ensure_live_turn(db, None, None, "assistant.service_failure")
                .await?;
            if !chat.turn_live || chat.turn_stopped {
                return Ok(());
            }
            if self.binding != connection_binding(db, &self.connection, &self.api_key_id).await? {
                return Ok(());
            }
            let fingerprint = digest(
                json!({"connection":self.connection,"operation":self.operation,"binding":self.binding}),
            );
            let bucket = digest(
                json!({"user":chat.user_id,"conversation":chat.conversation_id,
            "key":chat.api_key_id,"turn":chat.turn_id,"kind":"unavailable_bucket"}),
            );
            let now = bson::DateTime::now();
            let entry = doc! {"fingerprint":&fingerprint,"connection_id":&self.connection,"operation":&self.operation,
            "binding":&self.binding,"selection_operation": selection_fingerprint(&self.operation_identity), "evidence":evidence,"expires_at":bson::DateTime::from_chrono(Utc::now()+Duration::seconds(60))};
            // One atomic per-turn array. Deduplication, expiry removal and capacity
            // admission happen in the same update under concurrent failures.
            let update = vec![doc! {"$set": {
                "user_id":&chat.user_id,"conversation_id":&chat.conversation_id,"api_key_id":&chat.api_key_id,
                "turn_id":&chat.turn_id,"kind":"unavailable_bucket",
                "expires_at":bson::DateTime::from_chrono(Utc::now()+Duration::minutes(35)),
                "entries":{"$let":{"vars":{"remaining":{"$filter":{"input":{"$ifNull":["$entries",[]]},"as":"entry",
                    "cond":{"$and":[{"$gt":["$$entry.expires_at",now]},{"$ne":["$$entry.fingerprint",&fingerprint]}]}}}},
                    "in":{"$cond":[{"$lt":[{"$size":"$$remaining"},MAX_EVIDENCE as i64]},
                        {"$concatArrays":["$$remaining",{"$literal":[entry]}]},"$$remaining"]}}},
            }}];
            let transaction_db = crate::services::transaction_service::durable_database(db);
            let owned_db = transaction_db.clone();
            let owner = chat.user_id.clone();
            crate::services::service_history::transaction::run_send(
                &transaction_db,
                &(),
                move |tx, _| {
                    let (db, owner, bucket, update) = (
                        owned_db.clone(),
                        owner.clone(),
                        bucket.clone(),
                        update.clone(),
                    );
                    Box::pin(async move {
                        let result: AppResult<()> = async {
                            crate::services::erasure_admission::fence_in_session(&db, &owner, tx)
                                .await?;
                            db.collection::<Document>(COLLECTION)
                                .update_one(doc! {"_id":bucket}, update)
                                .upsert(true)
                                .session(&mut **tx)
                                .await?;
                            Ok(())
                        }
                        .await;
                        crate::services::api_key_mutation_service::transaction_result(result)
                    })
                },
            )
            .await
            .map_err(crate::services::api_key_mutation_service::map_transaction_error)?;
            Ok(())
        })
    }
}

/// Reuse only exact, still-current pre-dispatch evidence when the pool health
/// cooldown has not opened (or has already elapsed). Preserve the configured
/// retry trigger instead of presenting another consent card for the same outage.
pub(super) async fn pool_predispatch_failure(
    db: &Database,
    chat: &acks::ChatAuthority,
    connection: &str,
    binding: &str,
    operation: &Value,
) -> AppResult<Option<crate::errors::PoolAttemptTransportKind>> {
    if !chat.turn_live || chat.turn_stopped {
        return Ok(None);
    }
    let mut filter = scope(chat)?;
    filter.insert("kind", "unavailable_bucket");
    let Some(bucket) = db
        .collection::<Document>(COLLECTION)
        .find_one(filter)
        .await?
    else {
        return Ok(None);
    };
    let operation = digest(operation.clone());
    for entry in bucket
        .get_array("entries")
        .into_iter()
        .flatten()
        .filter_map(bson::Bson::as_document)
    {
        if entry.get_str("connection_id") == Ok(connection)
            && entry.get_str("binding") == Ok(binding)
            && entry.get_str("operation") == Ok(operation.as_str())
            && entry
                .get_datetime("expires_at")
                .is_ok_and(|at| *at > bson::DateTime::now())
        {
            return Ok(match entry.get_str("evidence") {
                Ok("pre_dispatch") => Some(crate::errors::PoolAttemptTransportKind::Connect),
                Ok("node_pre_dispatch") => {
                    Some(crate::errors::PoolAttemptTransportKind::NodeUnsent)
                }
                _ => None,
            });
        }
    }
    Ok(None)
}

pub fn observe_transport<'a>(
    db: &'a Database,
    chat: Option<&'a acks::ChatAuthority>,
    admission: Option<&'a Admission>,
    result: Result<reqwest::Response, crate::services::proxy_service::ForwardRequestError>,
) -> futures::future::BoxFuture<'a, AppResult<reqwest::Response>> {
    Box::pin(async move {
        if let (Some(chat), Some(admission)) = (chat, admission) {
            match &result {
                Ok(response) => {
                    admission
                        .response(db, chat, response.status().as_u16())
                        .await?
                }
                Err(error) => admission.transport_failure(db, chat, error).await?,
            }
        }
        result.map_err(crate::services::proxy_service::ForwardRequestError::into_app_error)
    })
}
