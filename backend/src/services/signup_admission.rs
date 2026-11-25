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
                for &(namespace, key, limit, window) in windows {
                    if !RateWindowStore::admit_in_session(
                        db, namespace, key, limit, window, session,
                    )
                    .await?
                    {
                        return Err(AppError::RateLimited);
                    }
                }
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

#[cfg(test)]
mod tests;
