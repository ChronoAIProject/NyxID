use std::{net::IpAddr, time::Duration};

use mongodb::Database;

use crate::{
    config::SignupLimits,
    errors::{AppError, AppResult},
    models::user::{COLLECTION_NAME as USERS, User},
};

use super::{coordination_service::RateWindowStore, transaction_service};

const HOUR: Duration = Duration::from_secs(3_600);
const DAY: Duration = Duration::from_secs(86_400);

#[derive(Clone, Copy)]
pub(crate) struct SignupAdmission {
    limits: SignupLimits,
    client_ip: IpAddr,
}

impl SignupAdmission {
    pub(crate) fn new(limits: SignupLimits, client_ip: IpAddr) -> Self {
        Self { limits, client_ip }
    }

    fn windows(self, ip: &str) -> [(&'static str, &str, u64, Duration); 3] {
        let [hour, day, global] = self.limits.caps().map(u64::from);
        [
            ("signup_ip_hour", ip, hour, HOUR),
            ("signup_ip_day", ip, day, DAY),
            ("signup_global_day", "all", global, DAY),
        ]
    }

    /// Check before the email duplicate shortcut, which must give the same
    /// admission response for known and unknown addresses.
    pub(crate) async fn check(&self, db: &Database) -> AppResult<()> {
        let ip = self.client_ip.to_string();
        for (namespace, key, limit, window) in self.windows(&ip) {
            if RateWindowStore::is_exhausted(db, namespace, key, limit, window).await? {
                return Err(AppError::RateLimited);
            }
        }
        Ok(())
    }

    /// Record all three windows and the person together. Duplicate inserts
    /// and aborted transactions leave every budget available to other signups.
    pub(crate) async fn insert_user(self, db: &Database, user: &User) -> AppResult<()> {
        let ip = self.client_ip.to_string();
        let context = (db, user, self.windows(&ip));
        transaction_service::run_db_only(db, &context, |session, (db, user, windows)| {
            Box::pin(async move {
                let [hour, ip_day, global_day] = *windows;
                // UTC day boundaries coincide with UTC hour boundaries. The
                // narrowest window supplies one MongoDB anchor for this attempt.
                let anchor = charge(db, session, hour, None).await?;
                charge(db, session, ip_day, Some(anchor)).await?;
                #[cfg(test)]
                tests::advance_between_daily_charges();
                charge(db, session, global_day, Some(anchor)).await?;
                db.collection::<User>(USERS)
                    .insert_one(*user)
                    .session(session)
                    .await?;
                Ok(())
            })
        })
        .await
    }
}

async fn charge(
    db: &Database,
    session: &mut mongodb::ClientSession,
    (namespace, key, limit, duration): (&str, &str, u64, Duration),
    anchor: Option<chrono::DateTime<chrono::Utc>>,
) -> AppResult<chrono::DateTime<chrono::Utc>> {
    RateWindowStore::admit_window_in_session(db, namespace, key, limit, duration, anchor, session)
        .await?
        .ok_or(AppError::RateLimited)
}

#[cfg(test)]
mod tests;
