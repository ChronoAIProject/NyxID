//! Local operator reconciliation. No vendor credentials are printed or stored in
//! jobs. Evidence references must identify an operator-held verification record.
use crate::errors::{AppError, AppResult};
use bson::{Document, doc};
use clap::{Args, Subcommand};
use futures::TryStreamExt;

#[derive(Args)]
pub struct ErasureArgs {
    #[command(subcommand)]
    action: Action,
}
#[derive(Subcommand)]
enum Action {
    /// Record provider processing-drain evidence for the account's latest
    /// admitted capture generation after local deletion and transport drain.
    AttestAccountDrained {
        user_id: String,
        #[arg(long)]
        operator: String,
        #[arg(long)]
        evidence: String,
    },
    /// Attest verified deployment history contains no telemetry destination.
    /// Requires local account removal and current capture/processing drain.
    AttestNoHistoricalDestinations {
        user_id: String,
        #[arg(long)]
        operator: String,
        #[arg(long)]
        evidence: String,
        #[arg(long)]
        reason: String,
    },
    /// Attest provider receipt/processing is drained after a capture timeout or
    /// process crash, using external evidence before vendor deletion resumes.
    AttestDispatchDrained {
        dispatch_id: String,
        #[arg(long)]
        operator: String,
        #[arg(long)]
        evidence: String,
    },
    /// Show bounded status, including unresolved historical destinations.
    Status {
        #[arg(long)]
        user_id: Option<String>,
        #[arg(long, default_value_t = 50, value_parser = clap::value_parser!(u32).range(1..=100))]
        limit: u32,
    },
    /// Supply a historical vendor person UUID verified in operator evidence.
    BindPerson {
        job_id: String,
        person_uuid: String,
        #[arg(long)]
        operator: String,
        #[arg(long)]
        evidence: String,
    },
    /// Attest that the destination has no person, events or recordings for this
    /// account after checking historical evidence, not just an empty lookup.
    AttestNoData {
        job_id: String,
        #[arg(long)]
        operator: String,
        #[arg(long)]
        evidence: String,
        #[arg(long)]
        reason: String,
    },
}
pub async fn run(db: &mongodb::Database, args: ErasureArgs) -> AppResult<()> {
    use crate::services::telemetry_erasure_service as service;
    match args.action {
        Action::AttestAccountDrained {
            user_id,
            operator,
            evidence,
        } => {
            crate::services::analytics_dispatch_service::attest_account_drained(
                db, &user_id, &operator, &evidence,
            )
            .await
        }
        Action::AttestNoHistoricalDestinations {
            user_id,
            operator,
            evidence,
            reason,
        } => {
            service::operator_reconcile_empty_inventory(db, &user_id, &operator, &evidence, &reason)
                .await
        }
        Action::AttestDispatchDrained {
            dispatch_id,
            operator,
            evidence,
        } => {
            crate::services::analytics_dispatch_service::attest_drained(
                db,
                &dispatch_id,
                &operator,
                &evidence,
            )
            .await
        }
        Action::Status { user_id, limit } => {
            let filter = user_id
                .as_ref()
                .map(|id| doc! {"user_id":id})
                .unwrap_or_default();
            let mut dispatch_filter = filter.clone();
            dispatch_filter.insert("status", doc! {"$in":["in_flight","uncertain"]});
            let rows: Vec<Document> = db
                .collection::<Document>(crate::models::telemetry_erasure_job::COLLECTION_NAME)
                .find(filter)
                .sort(doc! {"created_at":1})
                .limit(limit as i64)
                .projection(
                    doc! {"_id":1,"user_id":1,"destination_id":1,"status":1,"last_error":1,
                    "completion_basis":1,"inventory_revision":1,"retry_at":1},
                )
                .await?
                .try_collect()
                .await?;
            let dispatches: Vec<Document> = db
                .collection::<Document>(crate::services::analytics_dispatch_service::COLLECTION)
                .find(dispatch_filter)
                .sort(doc! {"created_at":1})
                .limit(limit as i64)
                .projection(doc! {"_id":1,"user_id":1,"status":1,"created_at":1})
                .await?
                .try_collect()
                .await?;
            let accounts: Vec<Document> = db
                .collection::<Document>(crate::services::analytics_dispatch_service::ACCOUNTS)
                .find(user_id.map(|id| doc! {"_id":id}).unwrap_or_default())
                .sort(doc! {"updated_at":1})
                .limit(limit as i64)
                .await?
                .try_collect()
                .await?;
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({"jobs":rows,"unsettled_dispatches":dispatches,"capture_accounts":accounts})
                )
                .map_err(|_| AppError::Internal("Status encoding failed".into()))?
            );
            Ok(())
        }
        Action::BindPerson {
            job_id,
            person_uuid,
            operator,
            evidence,
        } => {
            service::operator_reconcile(db, &job_id, Some(&person_uuid), &operator, &evidence, None)
                .await
        }
        Action::AttestNoData {
            job_id,
            operator,
            evidence,
            reason,
        } => {
            service::operator_reconcile(db, &job_id, None, &operator, &evidence, Some(&reason))
                .await
        }
    }
}
