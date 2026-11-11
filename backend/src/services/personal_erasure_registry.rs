//! Ownership registry for erasure bookkeeping. Provider receipts survive the
//! local inventory phase and are deleted only after every obligation settles.
use bson::{Document, doc};

pub(crate) const REVOCATION_RECEIPTS: &str = "personal_erasure_revocation_receipts";

pub(crate) async fn ensure_indexes(db: &mongodb::Database) -> crate::errors::AppResult<()> {
    db.collection::<Document>(REVOCATION_RECEIPTS)
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! {"user_id":1,"_id":1})
                .build(),
        )
        .await?;
    Ok(())
}

/// Delete: personal owner is the exact user_id. Final-source cleanup is the
/// only phase allowed to remove dedup receipts; source absence is not evidence.
pub(crate) fn final_metadata(owner: &str) -> [(&'static str, Document); 1] {
    [(REVOCATION_RECEIPTS, doc! {"user_id":owner})]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revocation_receipts_have_an_exact_owner_and_final_cleanup_phase() {
        assert_eq!(
            final_metadata("person"),
            [(REVOCATION_RECEIPTS, doc! {"user_id":"person"})]
        );
    }
}
