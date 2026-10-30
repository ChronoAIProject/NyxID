//! First-party owner reads/cancellation are independent of execution sources.
use super::*;
use nyxid_machine::authority::OwnerJobAccess;

impl Runtime {
    pub(super) fn owner_job_access(&self, request: &Request) -> Option<OwnerJobAccess> {
        if request.version != 1
            || request.authority.is_some()
            || request.external_authority.is_some()
            || !matches!(request.operation, Operation::Job | Operation::JobCancel)
        {
            return None;
        }
        let access: OwnerJobAccess =
            serde_json::from_value(request.parameters["owner_job_access"].clone()).ok()?;
        let clock = nyxid_machine::authority::LeaseClock::signed(
            request.timestamp,
            access.expires_at_ms,
            chrono::Utc::now().timestamp_millis(),
        )
        .ok()?;
        access
            .valid(
                &self.node_id,
                &self.runtime_id,
                request.parameters["job_id"].as_str()?,
                clock.now_ms,
            )
            .then_some(access)
    }

    /// Called only after the ordinary HMAC/replay verifier accepted the request.
    pub(super) async fn handle_owner_job(&self, request: Request) -> Value {
        let Some(access) = self.owner_job_access(&request) else {
            return json!({"error":{"code":12421,"message":"machine_authority_stale: owner job authority expired or invalid"}});
        };
        let clock = match nyxid_machine::authority::LeaseClock::signed(
            request.timestamp,
            access.expires_at_ms,
            chrono::Utc::now().timestamp_millis(),
        ) {
            Ok(clock) => clock,
            Err(error) => {
                return json!({"error":{"code":12421,"message":"machine_authority_stale: owner job timing refused","reason":error.reason()}});
            }
        };
        self.handle_owner_job_at(request, access, clock).await
    }

    pub(super) async fn handle_owner_job_at(
        &self,
        request: Request,
        access: OwnerJobAccess,
        clock: nyxid_machine::authority::LeaseClock,
    ) -> Value {
        {
            let mut deadlines = self
                .owner_job_deadlines
                .lock()
                .expect("owner job deadlines");
            deadlines.retain(|_, (_, deadline)| *deadline > std::time::Instant::now());
            if deadlines.len() >= 8192 {
                return json!({"error":{"code":12422,"message":"machine_authority_busy"}});
            }
            deadlines.insert(request.request_id.clone(), (access.clone(), clock.deadline));
        }
        if request.operation == Operation::JobCancel {
            if let Err(error) = self
                .jobs
                .cancel_owner(&access.job_id, &access.owner_id, clock.deadline)
                .await
            {
                let (code, message) = MachineError::from(error).public();
                return json!({"error":{"code":code,"message":message}});
            }
            if let Some(gateway) = self.gateway.get() {
                gateway
                    .cancel_jobs(std::slice::from_ref(&access.job_id))
                    .await;
            }
            return json!({"job_id":access.job_id,"cancel_requested":true});
        }
        if self
            .jobs
            .authorize_owner(&access.job_id, &access.owner_id)
            .await
            .is_err()
        {
            return json!({"error":{"code":12403,"message":"job_not_found"}});
        }
        let remaining = clock
            .deadline
            .saturating_duration_since(std::time::Instant::now())
            .as_millis() as i64;
        if remaining <= 0 {
            return json!({"error":{"code":12421,"message":"machine_authority_stale: owner job authority expired"}});
        }
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(remaining as u64),
            self.jobs.result(
                &access.job_id,
                request.parameters["wait_secs"]
                    .as_u64()
                    .unwrap_or(0)
                    .min(30),
                request.parameters["output_offset"].as_u64().unwrap_or(0),
                request.parameters["stderr_offset"].as_u64().unwrap_or(0),
            ),
        )
        .await;
        if clock.deadline <= std::time::Instant::now() {
            return json!({"error":{"code":12421,"message":"machine_authority_stale: owner job authority expired"}});
        }
        match result {
            Ok(Ok(mut value)) => {
                scrub_value(&mut value, &*self.redactor.lock().await);
                value
            }
            Ok(Err(error)) => {
                let (code, message) = MachineError::from(error).public();
                json!({"error":{"code":code,"message":message}})
            }
            Err(_) => {
                json!({"error":{"code":12421,"message":"machine_authority_stale: owner job authority expired"}})
            }
        }
    }
}
