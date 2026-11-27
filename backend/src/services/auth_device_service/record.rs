//! The two protocols share the session format. The collection that actually
//! owns the request is part of its identity, including when both IDs coincide.
use super::*;
use crate::crypto::record_field::{self, RecordContext};
use crate::models::auth_device_code::AuthDeviceAgentKeyGrant;
use bson::Document;

pub(super) const ACCESS: &str = "delivery_access_token_encrypted";
pub(super) const REFRESH: &str = "delivery_refresh_token_encrypted";
pub(super) const AGENT: &str = "agent_key_grant.delivery_credential_encrypted";
const VERSION: &str = "delivery_context_version";

pub(crate) const ISSUANCE_MISSING: &str = "auth_device_issuance_evidence_missing";

pub(super) fn approval_update(mut set: Document, contextual: bool) -> Document {
    if contextual {
        set.insert(VERSION, 3);
        doc! {"$set": set}
    } else {
        doc! {"$set": set, "$unset": {VERSION: ""}}
    }
}

fn invalid() -> AppError {
    AppError::Internal("Auth-device delivery requires authenticated repair".into())
}

pub(super) fn session_paths(session: &str) -> [[&str; 2]; 2] {
    [[ACCESS, session], [REFRESH, session]]
}

pub(super) fn grant_path<'a>(parent: &'a str, child: &'a str) -> [&'a str; 3] {
    [AGENT, parent, child]
}

pub(super) async fn seal(
    keys: &EncryptionKeys,
    collection: &str,
    id: &str,
    owner: &str,
    path: &[&str],
    value: &[u8],
) -> AppResult<Vec<u8>> {
    record_field::encrypt_context(
        keys,
        &RecordContext::new(collection, id, owner, path)?,
        value,
    )
    .await
}

fn context<'a>(
    collection: &'a str,
    row: &'a AuthDeviceCode,
    path: &'a [&'a str],
) -> AppResult<RecordContext<'a>> {
    RecordContext::new(
        collection,
        &row.id,
        row.approved_user_id.as_deref().ok_or_else(invalid)?,
        path,
    )
}

async fn open(
    keys: &EncryptionKeys,
    collection: &str,
    row: &AuthDeviceCode,
    path: &[&str],
    bytes: &[u8],
) -> AppResult<Zeroizing<String>> {
    let plain = record_field::decrypt_context(
        keys,
        &context(collection, row, path)?,
        row.delivery_context_version,
        bytes,
    )
    .await?;
    crate::crypto::guarded_utf8(plain).map_err(|_| {
        AppError::Internal(
            match path[0] {
                ACCESS => "auth-device delivery access token is not valid UTF-8",
                REFRESH => "auth-device delivery refresh token is not valid UTF-8",
                _ => "Invalid Agent Key delivery encoding",
            }
            .into(),
        )
    })
}

pub(super) async fn open_session(
    keys: &EncryptionKeys,
    collection: &str,
    row: &AuthDeviceCode,
) -> AppResult<(Zeroizing<String>, Zeroizing<String>)> {
    let session = row
        .approved_session_id
        .as_deref()
        .ok_or_else(|| AppError::Internal("Missing account session".into()))?;
    let [access, refresh] = session_paths(session);
    let missing = || AppError::Internal("Missing account delivery".into());
    Ok((
        open(
            keys,
            collection,
            row,
            &access,
            row.delivery_access_token_encrypted
                .as_deref()
                .ok_or_else(missing)?,
        )
        .await?,
        open(
            keys,
            collection,
            row,
            &refresh,
            row.delivery_refresh_token_encrypted
                .as_deref()
                .ok_or_else(missing)?,
        )
        .await?,
    ))
}

pub(super) async fn open_grant(
    keys: &EncryptionKeys,
    collection: &str,
    row: &AuthDeviceCode,
    grant: &AuthDeviceAgentKeyGrant,
) -> AppResult<Zeroizing<String>> {
    if collection != V2_COLLECTION_NAME
        || row.approved_session_id.is_some()
        || row.delivery_access_token_encrypted.is_some()
        || row.delivery_refresh_token_encrypted.is_some()
    {
        return Err(invalid());
    }
    open(
        keys,
        collection,
        row,
        &grant_path(&grant.api_key_id, &grant.credential_id),
        grant
            .delivery_credential_encrypted
            .as_deref()
            .ok_or(AppError::AgentKeyCredentialNotFound)?,
    )
    .await
}

struct Field<'a> {
    path: Vec<&'a str>,
    bytes: &'a [u8],
}

fn fields<'a>(collection: &str, row: &'a AuthDeviceCode) -> AppResult<Vec<Field<'a>>> {
    let session = row.approved_session_id.as_deref();
    let access = row.delivery_access_token_encrypted.as_deref();
    let refresh = row.delivery_refresh_token_encrypted.as_deref();
    let grant = row.agent_key_grant.as_ref();
    let child = grant.and_then(|g| g.delivery_credential_encrypted.as_deref());
    if row.delivery_context_version.is_some_and(|v| v != 3)
        || (grant.is_some()
            && (collection != V2_COLLECTION_NAME
                || session.is_some()
                || access.is_some()
                || refresh.is_some()))
    {
        return Err(invalid());
    }
    if access.is_none() && refresh.is_none() && child.is_none() {
        if row.delivery_context_version.is_some()
            || (row.status == AuthDeviceCodeStatus::Approved
                && (row.login_approval_id.is_none() || session.is_some() || grant.is_some()))
        {
            return Err(invalid());
        }
        return Ok(Vec::new());
    }
    if row.status != AuthDeviceCodeStatus::Approved || row.login_approval_id.is_some() {
        return Err(invalid());
    }
    match (session, access, refresh, grant, child) {
        (Some(session), Some(access), Some(refresh), None, None) => {
            let [a, r] = session_paths(session);
            Ok(vec![
                Field {
                    path: a.to_vec(),
                    bytes: access,
                },
                Field {
                    path: r.to_vec(),
                    bytes: refresh,
                },
            ])
        }
        (None, None, None, Some(grant), Some(bytes)) => Ok(vec![Field {
            path: grant_path(&grant.api_key_id, &grant.credential_id).to_vec(),
            bytes,
        }]),
        _ => Err(invalid()),
    }
}

pub(crate) fn validate_shape(collection: &str, source: &Document) -> AppResult<()> {
    let row = bson::from_document(source.clone()).map_err(|_| invalid())?;
    fields(collection, &row).map(drop)
}

pub(crate) async fn verify(
    keys: &EncryptionKeys,
    collection: &str,
    source: &Document,
) -> AppResult<()> {
    let row: AuthDeviceCode = bson::from_document(source.clone()).map_err(|_| invalid())?;
    for field in fields(collection, &row)? {
        if row.delivery_context_version != Some(3) {
            return Err(invalid());
        }
        keys.authenticate_current_record(field.bytes, &context(collection, &row, &field.path)?)
            .await?;
    }
    Ok(())
}

// Historical envelopes authenticated these bytes without a destination. Check
// their issuing identities before assigning a contextual envelope, even after expiry.
async fn check_legacy(
    db: &Database,
    row: &AuthDeviceCode,
    path: &[&str],
    value: &str,
) -> AppResult<()> {
    use crate::models::{api_key_credential, refresh_token};
    if path[0] == AGENT {
        let issued = db
            .collection::<Document>(api_key_credential::COLLECTION_NAME)
            .find_one(doc! {
                "_id": path[2],
            })
            .projection(doc! {"api_key_id": 1, "login_request_id": 1, "secret_hash": 1})
            .await?
            .ok_or_else(|| AppError::Conflict(ISSUANCE_MISSING.into()))?;
        return (issued.get_str("api_key_id") == Ok(path[1])
            && issued.get_str("login_request_id") == Ok(row.id.as_str())
            && issued.get_str("secret_hash")
                == Ok(crate::crypto::token::hash_token(value).as_str()))
        .then_some(())
        .ok_or_else(invalid);
    }
    let Some(jti) = issued_session(
        row.approved_user_id.as_deref().ok_or_else(invalid)?,
        path[0],
        path[1],
        value,
    )?
    else {
        return Ok(());
    };
    let refresh = db
        .collection::<Document>(refresh_token::COLLECTION_NAME)
        .find_one(doc! {
            "jti": jti.as_str(),
        })
        .projection(doc! {"user_id": 1, "session_id": 1})
        .await?
        .ok_or_else(|| AppError::Conflict(ISSUANCE_MISSING.into()))?;
    (refresh.get_str("user_id") == Ok(row.approved_user_id.as_deref().ok_or_else(invalid)?)
        && refresh.get_str("session_id") == Ok(path[1]))
    .then_some(())
    .ok_or_else(invalid)
}

pub(crate) fn issued_session(
    owner: &str,
    field: &str,
    session: &str,
    value: &str,
) -> AppResult<Option<Zeroizing<String>>> {
    #[derive(serde::Deserialize)]
    struct Issued<'a> {
        sub: &'a str,
        token_type: &'a str,
        sid: Option<&'a str>,
        jti: &'a str,
    }
    let encoded = value.split('.').nth(1).ok_or_else(invalid)?;
    let decoded = Zeroizing::new(URL_SAFE_NO_PAD.decode(encoded).map_err(|_| invalid())?);
    let claims: Issued<'_> = serde_json::from_slice(&decoded).map_err(|_| invalid())?;
    if claims.sub != owner {
        return Err(invalid());
    }
    if field == ACCESS {
        return (claims.token_type == "access" && claims.sid == Some(session))
            .then_some(None)
            .ok_or_else(invalid);
    }
    if claims.token_type != "refresh" {
        return Err(invalid());
    }
    Ok(Some(Zeroizing::new(claims.jti.to_owned())))
}

pub(crate) async fn migrate(
    db: &Database,
    keys: &EncryptionKeys,
    collection: &str,
    source: &Document,
) -> AppResult<bool> {
    let row: AuthDeviceCode = bson::from_document(source.clone()).map_err(|_| invalid())?;
    let fields = fields(collection, &row)?;
    if fields.is_empty() {
        return Ok(false);
    }
    if !keys.contextual_record_writes() {
        return Err(AppError::Conflict(
            "Contextual writer cutover is required before migration".into(),
        ));
    }
    if row.delivery_context_version == Some(3) && verify(keys, collection, source).await.is_ok() {
        return Ok(false);
    }
    let mut set = doc! {VERSION: 3};
    for field in fields {
        let plain = open(keys, collection, &row, &field.path, field.bytes).await?;
        if row.delivery_context_version.is_none() {
            check_legacy(db, &row, &field.path, &plain).await?;
        }
        let ctx = context(collection, &row, &field.path)?;
        let bytes = keys.encrypt_record(plain.as_bytes(), &ctx).await?;
        keys.authenticate_current_record(&bytes, &ctx).await?;
        set.insert(
            field.path[0],
            Binary {
                subtype: BinarySubtype::Generic,
                bytes,
            },
        );
    }
    Ok(crate::services::transaction_service::durable_database(db)
        .collection::<Document>(collection)
        .update_one(
            crate::services::record_storage::exact_document_predicate(source),
            doc! {"$set": set},
        )
        .await?
        .matched_count
        == 1)
}
