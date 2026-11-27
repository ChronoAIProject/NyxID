//! A clone retaining all database identities and evidence can be invisible.
//! Persist observable location changes until an administrator advances the epoch.
use super::*;

pub(super) async fn identity(db: &Database) -> AppResult<Document> {
    let mut id = transaction_service::replica_database_identity(db).await?;
    let mut catalog = db
        .list_collections()
        .filter(doc! {"name":{"$in":[HEAD,ENTRIES,DEPENDENCIES]}})
        .await?;
    while let Some(row) = catalog.try_next().await? {
        id.insert(row.name, row.info.uuid.ok_or_else(corrupt)?);
    }
    Ok(id)
}

pub(super) async fn observed(db: &Database) -> AppResult<Option<Document>> {
    let head = db.collection::<Document>(HEAD);
    let Some(row) = head.find_one(doc! {"_id":SINGLETON}).await? else {
        for collection in [
            db.collection::<Document>(ENTRIES),
            db.collection::<Document>(DEPENDENCIES),
        ] {
            if collection
                .find_one(doc! {})
                .projection(doc! {"_id":1})
                .await?
                .is_some()
            {
                return Err(conflict("Lifecycle records exist without their singleton"));
            }
        }
        return Ok(None);
    };
    if row.get_bool("restore_required") == Ok(true)
        || row.get_document("location").ok() == Some(&identity(db).await?)
    {
        return Ok(Some(row));
    }
    let request = Recording {
        request_id: fresh(),
        owner: "nyxid".into(),
        expected_revision: field(&row, "revision")?,
        evidence_ref: fresh(),
    };
    record(&Mutation {
        db,
        actor: None,
        scope: field(&row, "scope")?,
        digest: request.request_id.clone(),
        recording: &request,
        action: Action::Detected,
    })
    .await?;
    Ok(Some(
        head.find_one(doc! {"_id":SINGLETON})
            .await?
            .ok_or_else(stale)?,
    ))
}
