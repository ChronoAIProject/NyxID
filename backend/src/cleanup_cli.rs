use std::collections::HashSet;
use std::io::{self, BufRead, Write};

use clap::Args;
use futures::TryStreamExt;
use mongodb::bson::{Document, doc};

use crate::errors::{AppError, AppResult};
use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
use crate::models::user_api_key::COLLECTION_NAME as USER_API_KEYS;
use crate::models::user_endpoint::COLLECTION_NAME as USER_ENDPOINTS;
use crate::models::user_service::COLLECTION_NAME as USER_SERVICES;

#[derive(Args, Debug)]
pub struct CleanupArgs {
    /// Preview orphaned rows without deleting. Implies no prompt.
    #[arg(long)]
    pub dry_run: bool,

    /// Skip the confirmation prompt. A preview is still printed.
    #[arg(long, short = 'y')]
    pub yes: bool,
}

struct OrphanRow {
    id: String,
    user_id: String,
    label: String,
    extra: String,
}

struct OrphanReport {
    endpoints: Vec<OrphanRow>,
    api_keys: Vec<OrphanRow>,
}

pub async fn run(db: &mongodb::Database, args: CleanupArgs) -> AppResult<()> {
    println!("Scanning for orphaned user_endpoints and user_api_keys...");
    let report = collect_orphans(db).await?;

    if report.endpoints.is_empty() && report.api_keys.is_empty() {
        println!("No orphaned rows found. Nothing to do.");
        return Ok(());
    }

    let owners = load_owners(db, &report).await?;
    print_report(&report, &owners);

    if args.dry_run {
        println!("\nDry run: no rows were deleted.");
        return Ok(());
    }

    if !args.yes && !confirm().map_err(|e| AppError::Internal(format!("stdin read failed: {e}")))? {
        println!("Aborted. No rows were deleted.");
        return Ok(());
    }

    let endpoint_ids: Vec<&str> = report.endpoints.iter().map(|r| r.id.as_str()).collect();
    let api_key_ids: Vec<&str> = report.api_keys.iter().map(|r| r.id.as_str()).collect();

    let endpoint_deleted = if endpoint_ids.is_empty() {
        0
    } else {
        db.collection::<Document>(USER_ENDPOINTS)
            .delete_many(doc! { "_id": { "$in": &endpoint_ids } })
            .await?
            .deleted_count
    };
    let api_key_deleted = if api_key_ids.is_empty() {
        0
    } else {
        db.collection::<Document>(USER_API_KEYS)
            .delete_many(doc! { "_id": { "$in": &api_key_ids } })
            .await?
            .deleted_count
    };

    println!("Deleted {endpoint_deleted} user_endpoints, {api_key_deleted} user_api_keys.");
    Ok(())
}

async fn collect_orphans(db: &mongodb::Database) -> AppResult<OrphanReport> {
    let services: Vec<Document> = db
        .collection::<Document>(USER_SERVICES)
        .find(doc! { "is_active": true })
        .projection(doc! { "endpoint_id": 1, "api_key_id": 1 })
        .await?
        .try_collect()
        .await?;

    let mut active_endpoint_ids: HashSet<String> = HashSet::new();
    let mut active_api_key_ids: HashSet<String> = HashSet::new();
    for svc in &services {
        if let Ok(id) = svc.get_str("endpoint_id") {
            active_endpoint_ids.insert(id.to_string());
        }
        if let Ok(id) = svc.get_str("api_key_id") {
            active_api_key_ids.insert(id.to_string());
        }
    }

    let all_endpoints: Vec<Document> = db
        .collection::<Document>(USER_ENDPOINTS)
        .find(doc! {})
        .projection(doc! { "_id": 1, "user_id": 1, "label": 1, "url": 1 })
        .await?
        .try_collect()
        .await?;

    let endpoints: Vec<OrphanRow> = all_endpoints
        .into_iter()
        .filter_map(|row| {
            let id = row.get_str("_id").ok()?.to_string();
            if active_endpoint_ids.contains(&id) {
                return None;
            }
            Some(OrphanRow {
                id,
                user_id: row.get_str("user_id").unwrap_or("").to_string(),
                label: row.get_str("label").unwrap_or("").to_string(),
                extra: row.get_str("url").unwrap_or("").to_string(),
            })
        })
        .collect();

    let all_api_keys: Vec<Document> = db
        .collection::<Document>(USER_API_KEYS)
        .find(doc! {})
        .projection(doc! { "_id": 1, "user_id": 1, "label": 1, "status": 1 })
        .await?
        .try_collect()
        .await?;

    let api_keys: Vec<OrphanRow> = all_api_keys
        .into_iter()
        .filter_map(|row| {
            let id = row.get_str("_id").ok()?.to_string();
            if active_api_key_ids.contains(&id) {
                return None;
            }
            Some(OrphanRow {
                id,
                user_id: row.get_str("user_id").unwrap_or("").to_string(),
                label: row.get_str("label").unwrap_or("").to_string(),
                extra: row.get_str("status").unwrap_or("").to_string(),
            })
        })
        .collect();

    Ok(OrphanReport {
        endpoints,
        api_keys,
    })
}

struct OwnerInfo {
    user_type: UserType,
    display: String,
}

async fn load_owners(
    db: &mongodb::Database,
    report: &OrphanReport,
) -> AppResult<std::collections::HashMap<String, OwnerInfo>> {
    let mut ids: HashSet<String> = HashSet::new();
    for row in report.endpoints.iter().chain(report.api_keys.iter()) {
        if !row.user_id.is_empty() {
            ids.insert(row.user_id.clone());
        }
    }
    if ids.is_empty() {
        return Ok(Default::default());
    }
    let id_vec: Vec<&str> = ids.iter().map(String::as_str).collect();
    let users: Vec<User> = db
        .collection::<User>(USERS)
        .find(doc! { "_id": { "$in": &id_vec } })
        .await?
        .try_collect()
        .await?;
    let mut map = std::collections::HashMap::new();
    for u in users {
        let display = u
            .display_name
            .clone()
            .filter(|s| !s.is_empty())
            .unwrap_or(u.email.clone());
        map.insert(
            u.id,
            OwnerInfo {
                user_type: u.user_type,
                display,
            },
        );
    }
    Ok(map)
}

fn print_report(report: &OrphanReport, owners: &std::collections::HashMap<String, OwnerInfo>) {
    use std::collections::BTreeMap;
    let mut by_owner: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for r in &report.endpoints {
        by_owner.entry(r.user_id.clone()).or_default().0 += 1;
    }
    for r in &report.api_keys {
        by_owner.entry(r.user_id.clone()).or_default().1 += 1;
    }

    println!();
    println!(
        "Orphan user_endpoints: {}    Orphan user_api_keys: {}",
        report.endpoints.len(),
        report.api_keys.len()
    );
    println!();
    println!("Breakdown by owner:");
    println!(
        "  {:<38}  {:<6}  {:<40}  {:>9}  {:>8}",
        "user_id", "type", "display", "endpoints", "api_keys"
    );
    println!("  {}", "-".repeat(38 + 2 + 6 + 2 + 40 + 2 + 9 + 2 + 8));
    for (uid, (ep_count, ak_count)) in &by_owner {
        let (kind, display) = match owners.get(uid) {
            Some(info) => {
                let kind = if info.user_type.is_org() {
                    "org"
                } else {
                    "person"
                };
                (kind, info.display.as_str())
            }
            None => ("?", "(user not found)"),
        };
        let trimmed_display = if display.len() > 40 {
            &display[..40]
        } else {
            display
        };
        println!(
            "  {:<38}  {:<6}  {:<40}  {:>9}  {:>8}",
            uid, kind, trimmed_display, ep_count, ak_count
        );
    }

    let sample_n = 5usize;
    if !report.endpoints.is_empty() {
        println!();
        println!(
            "Sample orphan endpoints (first {} of {}):",
            sample_n.min(report.endpoints.len()),
            report.endpoints.len()
        );
        for r in report.endpoints.iter().take(sample_n) {
            println!(
                "  - id={} user={} label={:?} url={}",
                r.id, r.user_id, r.label, r.extra
            );
        }
    }
    if !report.api_keys.is_empty() {
        println!();
        println!(
            "Sample orphan api_keys (first {} of {}):",
            sample_n.min(report.api_keys.len()),
            report.api_keys.len()
        );
        for r in report.api_keys.iter().take(sample_n) {
            println!(
                "  - id={} user={} label={:?} status={}",
                r.id, r.user_id, r.label, r.extra
            );
        }
    }
}

fn confirm() -> io::Result<bool> {
    print!("\nProceed with hard-deletion? [y/N]: ");
    io::stdout().flush()?;
    let stdin = io::stdin();
    let mut line = String::new();
    stdin.lock().read_line(&mut line)?;
    let answer = line.trim().to_lowercase();
    Ok(answer == "y" || answer == "yes")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::connect_test_database;
    use chrono::Utc;

    #[tokio::test]
    async fn cleanup_deletes_rows_not_referenced_by_any_active_user_service() {
        let Some(db) = connect_test_database("cleanup_cli").await else {
            eprintln!("skipping cleanup_cli integration test: no local MongoDB available");
            return;
        };

        let now = bson::DateTime::from_chrono(Utc::now());
        let live_user = "user-live";
        let stuck_user = "user-stuck";

        // Live service (active) referencing live-ep / live-ak.
        db.collection::<Document>(USER_SERVICES)
            .insert_one(doc! {
                "_id": "svc-live",
                "user_id": live_user,
                "endpoint_id": "live-ep",
                "api_key_id": "live-ak",
                "is_active": true,
                "slug": "live",
                "auth_method": "bearer",
                "auth_key_name": "Authorization",
                "service_type": "http",
                "created_at": &now,
                "updated_at": &now,
            })
            .await
            .unwrap();
        // Stuck service (soft-deleted) referencing orphan-ep / orphan-ak.
        db.collection::<Document>(USER_SERVICES)
            .insert_one(doc! {
                "_id": "svc-stuck",
                "user_id": stuck_user,
                "endpoint_id": "orphan-ep",
                "api_key_id": "orphan-ak",
                "is_active": false,
                "slug": "stuck",
                "auth_method": "bearer",
                "auth_key_name": "Authorization",
                "service_type": "http",
                "created_at": &now,
                "updated_at": &now,
            })
            .await
            .unwrap();

        for (id, user) in [("live-ep", live_user), ("orphan-ep", stuck_user)] {
            db.collection::<Document>(USER_ENDPOINTS)
                .insert_one(doc! {
                    "_id": id,
                    "user_id": user,
                    "label": id,
                    "url": "https://example.com",
                    "created_at": &now,
                    "updated_at": &now,
                })
                .await
                .unwrap();
        }
        for (id, user) in [("live-ak", live_user), ("orphan-ak", stuck_user)] {
            db.collection::<Document>(USER_API_KEYS)
                .insert_one(doc! {
                    "_id": id,
                    "user_id": user,
                    "label": id,
                    "credential_type": "api_key",
                    "status": if id == "orphan-ak" { "revoked" } else { "active" },
                    "created_at": &now,
                    "updated_at": &now,
                })
                .await
                .unwrap();
        }

        let report = collect_orphans(&db).await.unwrap();
        let orphan_ep_ids: Vec<&str> = report.endpoints.iter().map(|r| r.id.as_str()).collect();
        let orphan_ak_ids: Vec<&str> = report.api_keys.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(orphan_ep_ids, vec!["orphan-ep"]);
        assert_eq!(orphan_ak_ids, vec!["orphan-ak"]);

        run(
            &db,
            CleanupArgs {
                dry_run: false,
                yes: true,
            },
        )
        .await
        .unwrap();

        // Live rows survive.
        assert_eq!(
            db.collection::<Document>(USER_ENDPOINTS)
                .count_documents(doc! { "_id": "live-ep" })
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            db.collection::<Document>(USER_API_KEYS)
                .count_documents(doc! { "_id": "live-ak" })
                .await
                .unwrap(),
            1
        );
        // Orphans are gone.
        assert_eq!(
            db.collection::<Document>(USER_ENDPOINTS)
                .count_documents(doc! { "_id": "orphan-ep" })
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            db.collection::<Document>(USER_API_KEYS)
                .count_documents(doc! { "_id": "orphan-ak" })
                .await
                .unwrap(),
            0
        );
        // Soft-deleted UserService tombstone is untouched.
        assert_eq!(
            db.collection::<Document>(USER_SERVICES)
                .count_documents(doc! { "_id": "svc-stuck" })
                .await
                .unwrap(),
            1
        );
    }
}
