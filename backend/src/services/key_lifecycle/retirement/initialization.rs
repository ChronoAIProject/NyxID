use super::*;

/// Read-only bootstrap proof for a database-using process.
pub(crate) struct Admission {
    pub(in crate::services::key_lifecycle) layout: Document,
    pub(crate) cohort: Option<String>,
}

pub(crate) struct Initialization(Option<String>);

impl Initialization {
    pub(crate) async fn begin(db: &Database, admission: Option<&Admission>) -> AppResult<Self> {
        let Some(admission) = admission else {
            return Ok(Self(None));
        };
        let id = fresh();
        let db = transaction_service::durable_database(db);
        let mut predicate = admission.layout.clone();
        predicate.insert("initializing.255", doc! {"$exists":false});
        let changed = db
            .collection::<Document>(HEAD)
            .update_one(
                predicate,
                doc! {"$inc":{"revision":1_i64},"$push":{"initializing":&id}},
            )
            .await?;
        if changed.matched_count != 1 {
            return Err(stale());
        }
        Ok(Self(Some(id)))
    }

    pub(crate) async fn finish(self, db: &Database) -> AppResult<()> {
        let Some(id) = self.0 else {
            return Ok(());
        };
        let db = transaction_service::durable_database(db);
        let changed = db
            .collection::<Document>(HEAD)
            .update_one(
                doc! {"_id":SINGLETON,"initializing":&id},
                doc! {"$pull":{"initializing":id},"$inc":{"revision":1_i64}},
            )
            .await?;
        if changed.matched_count != 1 {
            return Err(stale());
        }
        Ok(())
    }
}
