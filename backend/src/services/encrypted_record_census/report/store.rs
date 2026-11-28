//! Child partitions are immutable. The sealed checkpoint binds their ciphertext
//! hashes, so a losing worker can only leave unreachable, TTL-bounded staging.
use super::*;
use serde::de::DeserializeOwned;

pub(super) struct Store<'a> {
    pub db: &'a Database,
    pub keys: &'a EncryptionKeys,
    pub generation: &'a str,
}

impl Store<'_> {
    pub fn collection(&self) -> mongodb::Collection<Document> {
        self.db.collection(REPORTS)
    }

    pub async fn seal<T: Serialize>(&self, purpose: &str, value: &T) -> AppResult<bson::Binary> {
        let bytes = Zeroizing::new(bson::to_vec(value).map_err(|_| malformed())?);
        let bytes = self
            .keys
            .seal_census_record(self.generation, &["report-v1", purpose], &bytes)
            .await?;
        Ok(bson::Binary {
            subtype: bson::spec::BinarySubtype::Generic,
            bytes,
        })
    }

    pub async fn open<T: DeserializeOwned>(&self, purpose: &str, bytes: &[u8]) -> AppResult<T> {
        let fields = ["report-v1", purpose];
        let plain = self
            .keys
            .open_census_record(self.generation, &fields, bytes)
            .await
            .map_err(|_| malformed())?;
        bson::from_slice(&plain).map_err(|_| malformed())
    }

    pub async fn put<T: Serialize>(
        &self,
        purpose: &str,
        value: &T,
        deadline: bson::DateTime,
    ) -> AppResult<String> {
        let partition = self.seal(purpose, value).await?;
        let hash = hex::encode(Sha256::digest(&partition.bytes));
        self.collection().insert_one(doc!{"_id":{"generation":self.generation,"hash":&hash},"partition":partition,"expires_at":deadline}).await?;
        Ok(hash)
    }

    pub async fn get<T: DeserializeOwned>(&self, purpose: &str, hash: &str) -> AppResult<T> {
        let doc = self
            .collection()
            .find_one(doc! {"_id":{"generation":self.generation,"hash":hash}})
            .await?
            .ok_or_else(malformed)?;
        let bytes = doc
            .get_binary_generic("partition")
            .map_err(|_| malformed())?;
        if hex::encode(Sha256::digest(bytes)) != hash {
            return Err(malformed());
        }
        self.open(purpose, bytes).await
    }

    pub async fn load(&self) -> AppResult<(Document, Checkpoint)> {
        let doc = self
            .collection()
            .find_one(doc! {"_id":self.generation})
            .await?
            .ok_or_else(|| AppError::NotFound("Census generation unavailable".into()))?;
        let state: Checkpoint = self
            .open(
                "checkpoint",
                doc.get_binary_generic("checkpoint")
                    .map_err(|_| malformed())?,
            )
            .await?;
        if doc.get_i64("revision") != Ok(state.revision)
            || doc.get_datetime("expires_at") != Ok(&state.expires_at)
        {
            return Err(malformed());
        }
        Ok((doc, state))
    }

    pub async fn claim(&self, previous: &Document) -> AppResult<String> {
        let claim = Uuid::new_v4().to_string();
        let changed=self.collection().update_one(doc!{"_id":self.generation,"checkpoint":previous.get("checkpoint"),
            "$expr":{"$and":[{"$lte":["$lease_until","$$NOW"]},{"$gt":["$expires_at","$$NOW"]}]}},
            vec![doc!{"$set":{"claim":&claim,"lease_until":{"$add":["$$NOW",60_000]}}}]).await?;
        if changed.matched_count != 1 {
            return Err(refused("Census report lease unavailable"));
        }
        Ok(claim)
    }

    pub async fn commit(
        &self,
        census: &Census<'_>,
        claim: &str,
        previous: &Document,
        state: &mut Checkpoint,
    ) -> AppResult<()> {
        state.revision = state.revision.checked_add(1).ok_or_else(malformed)?;
        let sealed = self.seal("checkpoint", state).await?;
        let mut tx = self.db.client().start_session().await?;
        tx.start_transaction()
            .read_concern(ReadConcern::snapshot())
            .write_concern(WriteConcern::majority())
            .await?;
        // Write the same journal document as invalidation. A gap committed
        // during staging must win or cause this transaction to conflict.
        let fenced = census
            .log()
            .update_one(
                doc! {"_id":self.generation,"invalid":{"$exists":false},
                "scope_revision":state.ring.revision, "catalog":&state.catalog,"database_identity":&state.database_identity,"$expr":{"$gt":["$expires_at","$$NOW"]}},
                doc! {"$set":{"report_revision":state.revision}},
            )
            .session(&mut tx)
            .await?;
        if fenced.matched_count != 1 {
            return Err(refused("Census journal unavailable"));
        }
        let changed=self.collection().update_one(doc!{"_id":self.generation,"claim":claim,"revision":state.revision-1,
            "checkpoint":previous.get("checkpoint"),"$expr":{"$and":[{"$gt":["$lease_until","$$NOW"]},{"$gt":["$expires_at","$$NOW"]}]}},
            doc!{"$set":{"revision":state.revision,"checkpoint":sealed,"lease_until":bson::DateTime::from_millis(0)},"$unset":{"claim":""}}).session(&mut tx).await?;
        if changed.matched_count != 1 {
            return Err(refused("Census report lease changed"));
        }
        transaction_service::commit(&mut tx).await
    }
}
