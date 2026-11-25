use super::inbox::{self, InboxError};
use crate::models::{
    sandbox_usage_inbox::{PRODUCERS, Producer},
    service_account::{COLLECTION_NAME as ACCOUNTS, ServiceAccount, ServiceAccountPurpose},
    service_account_token::{COLLECTION_NAME as TOKENS, ServiceAccountToken},
};
use crate::services::audit_service::{self, AuditActor};
use bson::{DateTime, Document, doc};
use mongodb::{
    ClientSession, Database,
    options::{Acknowledgment, ReadConcern, WriteConcern},
};

pub const WRITE_SCOPE: &str = "billing:sandbox-usage:write";
pub const READ_SCOPE: &str = "billing:sandbox-usage:read";

pub struct Registration<'a> {
    pub account_id: &'a str,
    pub namespace: &'a str,
    pub owner_issuer: &'a str,
    pub expected_revision: i64,
    pub enabled: bool,
    pub accepting: bool,
}

pub async fn register(
    db: &Database,
    request: Registration<'_>,
    actor: &AuditActor,
    audit_key: &[u8],
) -> Result<i64, InboxError> {
    if request.expected_revision < 0 || (request.accepting && !request.enabled) {
        return Err(InboxError::Invalid("invalid_producer_state"));
    }
    super::policy::text(request.namespace).map_err(|_| InboxError::Invalid("invalid_namespace"))?;
    super::policy::text(request.owner_issuer).map_err(|_| InboxError::Invalid("invalid_issuer"))?;
    inbox::require_prepared(db).await?;
    let revision = request
        .expected_revision
        .checked_add(1)
        .ok_or(InboxError::Invalid("revision_overflow"))?;
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .read_concern(ReadConcern::snapshot())
        .write_concern(WriteConcern::builder().w(Acknowledgment::Majority).build())
        .await?;
    let result = async {
        register_in_session(db, &mut session, &request, revision).await?;
        audit_service::log_actor_event_in_session(
            db,
            &mut session,
            audit_key,
            actor,
            "sandbox_usage_producer_registered",
            serde_json::json!({
                "service_account_id": request.account_id,
                "namespace": request.namespace,
                "revision": revision,
                "enabled": request.enabled,
                "accepting": request.accepting,
            }),
        )
        .await
        .map_err(|_| InboxError::Unavailable)
    }
    .await;
    if let Err(error) = result {
        let _ = session.abort_transaction().await;
        return Err(error);
    }
    inbox::commit(&mut session).await?;
    Ok(revision)
}

async fn register_in_session(
    db: &Database,
    session: &mut ClientSession,
    request: &Registration<'_>,
    revision: i64,
) -> Result<(), InboxError> {
    // Read before taking document locks, including when disabling registration
    // after the producer account has been removed.
    let account = db
        .collection::<ServiceAccount>(ACCOUNTS)
        .find_one(doc! {"_id":request.account_id})
        .max_time(std::time::Duration::from_secs(5))
        .session(&mut *session)
        .await?;
    if request.enabled {
        let account = account.ok_or(InboxError::Invalid("service_account_missing"))?;
        if !account.is_active
            || account.purpose != ServiceAccountPurpose::General
            || !has_scope(&account.allowed_scopes, WRITE_SCOPE)
            || !has_scope(&account.allowed_scopes, READ_SCOPE)
        {
            return Err(InboxError::Invalid("service_account_scope"));
        }
        // This account can submit usage for unrelated owners in the namespace.
        // Protect its credentials in the same transaction that grants access.
        let protected = db
            .collection::<Document>(ACCOUNTS)
            .update_one(
                doc! {"_id":request.account_id},
                doc! {"$set":{"platform_protected":true}, "$inc":{"usage_custody_revision":1_i64}},
            )
            .session(&mut *session)
            .await?;
        if protected.matched_count != 1 {
            return Err(InboxError::Unavailable);
        }
    }
    let producers = db.collection::<Producer>(PRODUCERS);
    if request.expected_revision == 0 {
        let result = producers
            .insert_one(Producer {
                service_account_id: request.account_id.into(),
                namespace: request.namespace.into(),
                owner_issuer: request.owner_issuer.into(),
                enabled: request.enabled,
                accepting: request.accepting,
                revision,
            })
            .session(&mut *session)
            .await;
        return match result {
            Ok(_) => Ok(()),
            Err(error) if super::super::ledger::is_duplicate_key_error(&error) => {
                Err(InboxError::Conflict("producer_exists"))
            }
            Err(error) => Err(error.into()),
        };
    }
    let updated = producers.update_one(
        doc! {"_id":request.account_id,"namespace":request.namespace,"owner_issuer":request.owner_issuer,
            "revision":request.expected_revision},
        doc! {"$set":{"enabled":request.enabled,"accepting":request.accepting,"revision":revision}},
    ).session(&mut *session).await?;
    if updated.matched_count != 1 {
        return Err(InboxError::Conflict("producer_registration_changed"));
    }
    Ok(())
}

pub struct Authority {
    producer: Producer,
    token_jti: String,
    scope: &'static str,
}

impl Authority {
    pub async fn load(
        db: &Database,
        actor_id: &str,
        token_jti: &str,
        token_scopes: &str,
        scope: &'static str,
    ) -> Result<Self, InboxError> {
        if token_jti.is_empty() || !has_scope(token_scopes, scope) {
            return Err(InboxError::Forbidden);
        }
        Ok(Self {
            producer: inbox::producer(db, actor_id).await?,
            token_jti: token_jti.into(),
            scope,
        })
    }

    pub fn namespace(&self) -> &str {
        &self.producer.namespace
    }

    pub(super) async fn authorize_read(&self, db: &Database) -> Result<(), InboxError> {
        self.require_scope(READ_SCOPE)?;
        let account = db
            .collection::<ServiceAccount>(ACCOUNTS)
            .find_one(doc! {"_id":&self.producer.service_account_id, "is_active":true})
            .read_concern(ReadConcern::majority())
            .await?
            .ok_or(InboxError::Forbidden)?;
        let token = db
            .collection::<ServiceAccountToken>(TOKENS)
            .find_one(self.token_filter())
            .read_concern(ReadConcern::majority())
            .await?
            .ok_or(InboxError::Forbidden)?;
        self.verify_grant(&account, &token)?;
        let current = inbox::producer(db, &self.producer.service_account_id).await?;
        if current.revision != self.producer.revision {
            return Err(InboxError::Forbidden);
        }
        Ok(())
    }

    pub(super) fn require_scope(&self, scope: &'static str) -> Result<(), InboxError> {
        if self.scope == scope {
            Ok(())
        } else {
            Err(InboxError::Forbidden)
        }
    }

    pub(super) fn owner_issuer(&self) -> &str {
        &self.producer.owner_issuer
    }

    pub(super) fn accepting(&self) -> bool {
        self.producer.accepting
    }

    pub(super) async fn fence(
        &self,
        db: &Database,
        session: &mut ClientSession,
    ) -> Result<(), InboxError> {
        let account_id = &self.producer.service_account_id;
        // The transaction starts with a bounded read. Cancellation during its
        // first driver command must not strand a document write lock.
        let account = db
            .collection::<ServiceAccount>(ACCOUNTS)
            .find_one(doc! {"_id":account_id,"is_active":true})
            .max_time(std::time::Duration::from_secs(5))
            .session(&mut *session)
            .await?
            .ok_or(InboxError::Forbidden)?;
        let token_filter = self.token_filter();
        let token = db
            .collection::<ServiceAccountToken>(TOKENS)
            .find_one(token_filter.clone())
            .session(&mut *session)
            .await?
            .ok_or(InboxError::Forbidden)?;
        self.verify_grant(&account, &token)?;
        for (collection, filter) in [
            (ACCOUNTS, doc! {"_id":account_id,"is_active":true}),
            (TOKENS, token_filter),
            (
                PRODUCERS,
                doc! {"_id":account_id,"enabled":true,"revision":self.producer.revision,
                "namespace":&self.producer.namespace,"owner_issuer":&self.producer.owner_issuer},
            ),
        ] {
            let matched = db
                .collection::<Document>(collection)
                .update_one(filter, doc! {"$inc":{"usage_custody_revision":1_i64}})
                .session(&mut *session)
                .await?
                .matched_count;
            if matched != 1 {
                return Err(InboxError::Forbidden);
            }
        }
        Ok(())
    }

    fn token_filter(&self) -> Document {
        doc! {"jti":&self.token_jti,"service_account_id":&self.producer.service_account_id,
        "revoked":false,"expires_at":{"$gt":DateTime::now()}}
    }

    fn verify_grant(
        &self,
        account: &ServiceAccount,
        token: &ServiceAccountToken,
    ) -> Result<(), InboxError> {
        if !account.platform_protected
            || account.purpose != ServiceAccountPurpose::General
            || account.credential_generation != token.credential_generation
            || !has_scope(&account.allowed_scopes, self.scope)
            || !has_scope(&token.scope, self.scope)
        {
            return Err(InboxError::Forbidden);
        }
        Ok(())
    }
}

fn has_scope(scopes: &str, required: &str) -> bool {
    scopes.split_whitespace().any(|scope| scope == required)
}
