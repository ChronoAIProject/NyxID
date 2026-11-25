use super::inbox::{self, InboxError};
use crate::models::{
    sandbox_usage_inbox::{PRODUCERS, Producer},
    service_account::{COLLECTION_NAME as ACCOUNTS, ServiceAccount, ServiceAccountPurpose},
    service_account_token::{COLLECTION_NAME as TOKENS, ServiceAccountToken},
};
use bson::{DateTime, Document, doc};
use mongodb::{
    ClientSession, Database,
    options::{Acknowledgment, WriteConcern},
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

pub async fn register(db: &Database, request: Registration<'_>) -> Result<i64, InboxError> {
    if request.expected_revision < 0 || (request.accepting && !request.enabled) {
        return Err(InboxError::Invalid("invalid_producer_state"));
    }
    super::policy::text(request.namespace).map_err(|_| InboxError::Invalid("invalid_namespace"))?;
    super::policy::text(request.owner_issuer).map_err(|_| InboxError::Invalid("invalid_issuer"))?;
    let account = db
        .collection::<ServiceAccount>(ACCOUNTS)
        .find_one(doc! {"_id":request.account_id})
        .await?
        .ok_or(InboxError::Invalid("service_account_missing"))?;
    if request.enabled
        && (!account.is_active
            || account.purpose != ServiceAccountPurpose::General
            || !has_scope(&account.allowed_scopes, WRITE_SCOPE)
            || !has_scope(&account.allowed_scopes, READ_SCOPE))
    {
        return Err(InboxError::Invalid("service_account_scope"));
    }
    let producers = db.collection::<Producer>(PRODUCERS);
    inbox::require_prepared(db).await?;
    let write_concern = WriteConcern::builder().w(Acknowledgment::Majority).build();
    let revision = request
        .expected_revision
        .checked_add(1)
        .ok_or(InboxError::Invalid("revision_overflow"))?;
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
            .write_concern(write_concern)
            .await;
        return match result {
            Ok(_) => Ok(revision),
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
    ).write_concern(write_concern).await?;
    if updated.matched_count != 1 {
        return Err(InboxError::Conflict("producer_registration_changed"));
    }
    Ok(revision)
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
        let token_filter = doc! {"jti":&self.token_jti,"service_account_id":account_id,
        "revoked":false,"expires_at":{"$gt":DateTime::now()}};
        let token = db
            .collection::<ServiceAccountToken>(TOKENS)
            .find_one(token_filter.clone())
            .session(&mut *session)
            .await?
            .ok_or(InboxError::Forbidden)?;
        if account.purpose != ServiceAccountPurpose::General
            || account.credential_generation != token.credential_generation
            || !has_scope(&account.allowed_scopes, self.scope)
            || !has_scope(&token.scope, self.scope)
        {
            return Err(InboxError::Forbidden);
        }
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
}

fn has_scope(scopes: &str, required: &str) -> bool {
    scopes.split_whitespace().any(|scope| scope == required)
}
