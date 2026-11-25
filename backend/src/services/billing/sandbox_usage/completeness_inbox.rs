//! Durable custody for the separate minute-completeness statement protocol.
use super::{
    completeness_contract::{Basis, Statement},
    completeness_validation::{self, Decision},
    inbox::{self, InboxError},
    inbox_authority::{Authority, WRITE_SCOPE},
    strict_json,
};
use crate::models::sandbox_usage_inbox::{COMPLETENESS, Completeness};
use bson::{Binary, Document, doc, spec::BinarySubtype};
use chrono::Utc;
use mongodb::{
    ClientSession, Database, IndexModel,
    options::{Acknowledgment, IndexOptions, ReadConcern, WriteConcern},
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

fn definitions() -> [(Document, bool); 3] {
    [
        (doc! {"namespace":1,"statement_id":1}, true),
        (
            doc! {"namespace":1,"sandbox_id":1,"source_epoch":1,"basis":1,"minute_start_ms":1,"revision":1},
            true,
        ),
        (doc! {"owner_subject":1}, false),
    ]
}

pub async fn prepare(db: &Database) -> Result<(), InboxError> {
    let models = definitions().into_iter().map(|(keys, unique)| {
        IndexModel::builder()
            .keys(keys)
            .options(IndexOptions::builder().unique(unique).build())
            .build()
    });
    db.collection::<Completeness>(COMPLETENESS)
        .create_indexes(models)
        .await?;
    Ok(())
}

async fn require_prepared(db: &Database) -> Result<(), InboxError> {
    use futures::TryStreamExt;
    let mut remaining = definitions().to_vec();
    let collection = db.collection::<Completeness>(COMPLETENESS);
    let mut indexes = collection.list_indexes().await.map_err(|error| {
        if matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Command(command) if command.code == 26) {
            InboxError::Unavailable
        } else { error.into() }
    })?;
    while let Some(index) = indexes.try_next().await? {
        let Some(options) = index.options else {
            continue;
        };
        if options.expire_after.is_some() {
            return Err(InboxError::Unavailable);
        }
        if options.sparse.unwrap_or_default() || options.partial_filter_expression.is_some() {
            continue;
        }
        remaining.retain(|(keys, unique)| {
            *keys != index.keys || *unique != options.unique.unwrap_or_default()
        });
    }
    if remaining.is_empty() {
        Ok(())
    } else {
        Err(InboxError::Unavailable)
    }
}

#[derive(Serialize)]
pub struct Receipt {
    statement_id: String,
    receipt_id: String,
    statement_digest: String,
    accepted_at_ms: String,
    custody: &'static str,
}

#[derive(Serialize)]
pub struct Observation {
    pub receipt: Receipt,
    pub validation: Decision,
}

fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn decode(bytes: &[u8]) -> Result<(Statement, Vec<u8>, String), InboxError> {
    let value = strict_json::parse(bytes).map_err(InboxError::Invalid)?;
    let statement = Statement::from_value(value).map_err(InboxError::Invalid)?;
    let canonical = serde_json_canonicalizer::to_vec(&statement)
        .map_err(|_| InboxError::Invalid("canonical_statement"))?;
    let hash = digest(&canonical);
    Ok((statement, canonical, hash))
}

fn scope(statement: &Statement) -> Document {
    doc! {"namespace":&statement.producer_namespace,"sandbox_id":&statement.sandbox_id,
    "source_epoch":&statement.source_epoch,"basis":basis(statement.basis),"minute_start_ms":statement.minute_start_ms.0 as i64}
}

fn basis(basis: Basis) -> &'static str {
    match basis {
        Basis::Allocated => "allocated",
        Basis::Measured => "measured",
    }
}

fn decode_row(row: &Completeness) -> Result<Statement, InboxError> {
    let (statement, canonical, hash) =
        decode(&row.canonical.bytes).map_err(|_| InboxError::Corrupt)?;
    if row.digest != hash
        || canonical != row.canonical.bytes
        || row.statement_id != statement.statement_id
        || row.namespace != statement.producer_namespace
        || row.sandbox_id != statement.sandbox_id
        || row.source_epoch != statement.source_epoch
        || row.owner_subject != statement.owner.subject
        || row.basis != basis(statement.basis)
        || row.minute_start_ms != statement.minute_start_ms.0 as i64
        || row.revision != statement.revision.0 as i64
    {
        return Err(InboxError::Corrupt);
    }
    Ok(statement)
}

fn receipt(row: &Completeness, validation: Decision) -> Observation {
    Observation {
        receipt: Receipt {
            statement_id: row.statement_id.clone(),
            receipt_id: row.receipt_id.clone(),
            statement_digest: row.digest.clone(),
            accepted_at_ms: row.accepted_at.timestamp_millis().to_string(),
            custody: "persisted",
        },
        validation,
    }
}

pub async fn accept(
    db: &Database,
    authority: &Authority,
    bytes: &[u8],
) -> Result<(bool, Observation), InboxError> {
    authority.require_scope(WRITE_SCOPE)?;
    require_prepared(db).await?;
    let (statement, canonical, hash) = decode(bytes)?;
    if statement.capture_through_ms.0 > inbox::clock().now_ms {
        return Err(InboxError::Invalid("future_statement_boundary"));
    }
    if statement.producer_namespace != authority.namespace()
        || statement.owner.issuer != authority.owner_issuer()
    {
        return Err(InboxError::Forbidden);
    }
    if !statement.owner.subject.bytes().all(|byte| {
        byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':' | b'.' | b'|')
    }) {
        return Err(InboxError::Invalid("nonopaque_owner_reference"));
    }
    for _ in 0..4 {
        let mut session = db.client().start_session().await?;
        session
            .start_transaction()
            .read_concern(ReadConcern::snapshot())
            .write_concern(WriteConcern::builder().w(Acknowledgment::Majority).build())
            .await?;
        let result = async {
            authority.fence(db, &mut session).await?;
            let result = append(
                db,
                &mut session,
                &statement,
                &canonical,
                &hash,
                authority.accepting(),
            )
            .await?;
            inbox::commit(&mut session).await?;
            Ok::<_, InboxError>(result)
        }
        .await;
        match result {
            Ok(value) => return Ok(value),
            Err(InboxError::Database(error))
                if error.contains_label("UnknownTransactionCommitResult") =>
            {
                let observed =
                    read_namespace(db, authority.namespace(), &statement.statement_id).await?;
                return match observed {
                    Some(value) if value.receipt.statement_digest == hash => Ok((false, value)),
                    Some(_) => Err(InboxError::Conflict("statement_digest_changed")),
                    None => Err(InboxError::Unavailable),
                };
            }
            Err(error) => {
                let _ = session.abort_transaction().await;
                if !matches!(&error, InboxError::Database(error) if error.contains_label("TransientTransactionError") || super::super::ledger::is_duplicate_key_error(error))
                {
                    return Err(error);
                }
            }
        }
    }
    Err(InboxError::Unavailable)
}

async fn append(
    db: &Database,
    session: &mut ClientSession,
    statement: &Statement,
    canonical: &[u8],
    hash: &str,
    accepting: bool,
) -> Result<(bool, Observation), InboxError> {
    let rows = db.collection::<Completeness>(COMPLETENESS);
    if let Some(row) = rows
        .find_one(
            doc! {"namespace":&statement.producer_namespace,"statement_id":&statement.statement_id},
        )
        .session(&mut *session)
        .await?
    {
        if row.digest != hash || row.canonical.bytes != canonical {
            return Err(InboxError::Conflict("statement_digest_changed"));
        }
        let original = decode_row(&row)?;
        let validation = current_decision(db, session, &row, &original).await?;
        return Ok((false, receipt(&row, validation)));
    }
    if !accepting {
        return Err(InboxError::Unavailable);
    }
    let previous = rows
        .find_one(scope(statement))
        .sort(doc! {"revision":-1})
        .session(&mut *session)
        .await?;
    match previous.as_ref() {
        None if statement.revision.0 != 1 => {
            return Err(InboxError::Prerequisite("statement_predecessor_missing"));
        }
        Some(previous) => {
            let prior = decode_row(previous)?;
            if prior.owner != statement.owner
                || prior.revision.0.checked_add(1) != Some(statement.revision.0)
                || statement.previous_statement_digest.as_deref() != Some(previous.digest.as_str())
            {
                return Err(InboxError::Conflict("statement_predecessor_changed"));
            }
        }
        None => {}
    }
    let validation = completeness_validation::observe(db, session, statement).await?;
    let row = Completeness {
        id: Uuid::new_v4().to_string(),
        namespace: statement.producer_namespace.clone(),
        sandbox_id: statement.sandbox_id.clone(),
        source_epoch: statement.source_epoch.clone(),
        owner_subject: statement.owner.subject.clone(),
        basis: basis(statement.basis).into(),
        minute_start_ms: statement.minute_start_ms.0 as i64,
        revision: statement.revision.0 as i64,
        statement_id: statement.statement_id.clone(),
        digest: hash.into(),
        canonical: Binary {
            subtype: BinarySubtype::Generic,
            bytes: canonical.to_vec(),
        },
        receipt_id: Uuid::new_v4().to_string(),
        accepted_at: Utc::now(),
        accepted_validation: bson::to_document(&validation).map_err(|_| InboxError::Corrupt)?,
    };
    let observation = receipt(&row, validation);
    let _ = rows.insert_one(row).session(&mut *session).await?;
    Ok((true, observation))
}

async fn current_decision(
    db: &Database,
    session: &mut ClientSession,
    row: &Completeness,
    statement: &Statement,
) -> Result<Decision, InboxError> {
    let newer = db
        .collection::<Completeness>(COMPLETENESS)
        .find_one(scope(statement))
        .sort(doc! {"revision":-1})
        .session(&mut *session)
        .await?
        .ok_or(InboxError::Corrupt)?;
    if newer.revision > row.revision {
        return Ok(Decision::Superseded("statement_revised"));
    }
    completeness_validation::observe(db, session, statement).await
}

async fn read_namespace(
    db: &Database,
    namespace: &str,
    id: &str,
) -> Result<Option<Observation>, InboxError> {
    let mut session = db.client().start_session().snapshot(true).await?;
    let Some(row) = db
        .collection::<Completeness>(COMPLETENESS)
        .find_one(doc! {"namespace":namespace,"statement_id":id})
        .session(&mut session)
        .await?
    else {
        return Ok(None);
    };
    let statement = decode_row(&row)?;
    let validation = current_decision(db, &mut session, &row, &statement).await?;
    Ok(Some(receipt(&row, validation)))
}

pub async fn read(
    db: &Database,
    authority: &Authority,
    id: &str,
) -> Result<Option<Observation>, InboxError> {
    authority.authorize_read(db).await?;
    read_namespace(db, authority.namespace(), id).await
}

pub(super) async fn latest(
    db: &Database,
    session: &mut ClientSession,
    namespace: &str,
    sandbox: &str,
    epoch: &str,
    minute: i64,
    basis: Basis,
) -> Result<Option<(Statement, String, Decision)>, InboxError> {
    let Some(row) = db
        .collection::<Completeness>(COMPLETENESS)
        .find_one(doc! {"namespace":namespace,"sandbox_id":sandbox,
        "source_epoch":epoch,"minute_start_ms":minute,"basis":self::basis(basis)})
        .sort(doc! {"revision":-1})
        .session(&mut *session)
        .await?
    else {
        return Ok(None);
    };
    let statement = decode_row(&row)?;
    let validation = completeness_validation::observe(db, session, &statement).await?;
    Ok(Some((statement, row.digest, validation)))
}
