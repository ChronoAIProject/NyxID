use super::*;
use nyxid_machine::authority::ExternalAuthority;

impl Runtime {
    pub fn bind_managed_runtime(&self, binding: &nyxid_machine::managed::Binding) -> Result<()> {
        binding.validate().map_err(anyhow::Error::msg)?;
        anyhow::ensure!(binding.node_id == self.node_id, "managed Node mismatch");
        if let Some(current) = self.managed_binding.get() {
            anyhow::ensure!(current == binding, "managed runtime binding changed");
        } else {
            self.managed_binding
                .set(binding.clone())
                .map_err(|_| anyhow::anyhow!("managed runtime already bound"))?;
        }
        Ok(())
    }

    pub(super) fn external_envelope_valid(&self, request: &Request, _replay: bool) -> bool {
        let Some(authority) = request.external_authority.as_ref() else {
            return false;
        };
        if request.authority.is_some() {
            return false;
        }
        let Ok(clock) = nyxid_machine::authority::LeaseClock::signed(
            request.timestamp,
            authority.expires_at_ms,
            chrono::Utc::now().timestamp_millis(),
        ) else {
            return false;
        };
        let now = clock.now_ms;
        let stop = matches!(
            request.operation,
            Operation::AuthorityRevoke | Operation::JobCancel
        );
        (if stop {
            authority.valid_stop(&self.node_id, &self.runtime_id, now)
        } else {
            authority.valid(&self.node_id, &self.runtime_id, now)
        }) && (stop || !self.external_authority.revoked(&authority.lease_id))
            && match self.managed_binding.get() {
                Some(binding) => {
                    authority.runtime_generation == Some(binding.runtime_generation)
                        && authority.credential_epoch == Some(binding.credential_epoch)
                }
                None => {
                    authority.runtime_generation.is_none() && authority.credential_epoch.is_none()
                }
            }
            && (matches!(
                request.operation,
                Operation::AuthorityRenew | Operation::AuthorityRevoke
            ) || match request.operation {
                Operation::Exec | Operation::Job | Operation::JobCancel => {
                    authority.job_id.as_deref() == request.parameters["job_id"].as_str()
                        && authority.job_id.is_some()
                }
                Operation::ListFiles
                | Operation::ReadFile
                | Operation::WriteFile
                | Operation::EditFile => authority.job_id.is_none(),
                _ => false,
            })
    }

    pub(super) async fn handle_external(&self, mut request: Request) -> Value {
        if !self.external_envelope_valid(&request, false) {
            return json!({"error":{"code":12421,"message":"machine_authority_stale: external envelope or runtime changed"}});
        }
        let authority = request
            .external_authority
            .take()
            .expect("validated external authority");
        let clock = match nyxid_machine::authority::LeaseClock::signed(
            request.timestamp,
            authority.expires_at_ms,
            chrono::Utc::now().timestamp_millis(),
        ) {
            Ok(clock) => clock,
            Err(error) => {
                return json!({"error":{"code":12421,"message":"machine_authority_stale: external timing refused","reason":error.reason()}});
            }
        };
        let control = match request.operation {
            Operation::AuthorityRenew => Some(self.external_authority.renew_at(
                &authority,
                &self.node_id,
                &self.runtime_id,
                clock,
            )),
            Operation::AuthorityRevoke => Some(self.external_authority.revoke_at(
                &authority,
                &self.node_id,
                &self.runtime_id,
                clock,
            )),
            _ => None,
        };
        if let Some(result) = control {
            return match result {
                Ok(()) => json!({"accepted":true}),
                Err(_) => {
                    json!({"error":{"code":12421,"message":"machine_authority_stale: external lease refused"}})
                }
            };
        }
        if !authority
            .capabilities
            .allows(request.operation, &request.parameters)
            || (matches!(request.operation, Operation::Exec | Operation::Job) && !self.config.shell)
            || (matches!(
                request.operation,
                Operation::ListFiles
                    | Operation::ReadFile
                    | Operation::WriteFile
                    | Operation::EditFile
            ) && !self.config.files)
            || request.parameters.get("conversation_id").is_some()
            || request.parameters.get("turn_id").is_some()
        {
            return json!({"error":{"code":12420,"message":"machine_permission_revoked: external capability refused"}});
        }
        if request.operation == Operation::JobCancel {
            let id = authority.job_id.as_deref().expect("validated job");
            if self.jobs.authorize_external(id, &authority).await.is_err() {
                return json!({"error":{"code":12403,"message":"job_not_found"}});
            }
            let _ = self.jobs.cancel(id).await;
            self.jobs.preempt_ids(&[id.to_owned()]).await;
            if let Some(gateway) = self.gateway.get() {
                gateway.cancel_jobs(&[id.to_owned()]).await;
            }
            return json!({"job_id":id,"cancel_requested":true});
        }
        let mut stopped = match self.external_authority.admit_at(
            &authority,
            &self.node_id,
            &self.runtime_id,
            request.operation == Operation::Exec,
            clock,
        ) {
            Ok(receiver) => receiver,
            Err(_) => {
                return json!({"error":{"code":12421,"message":"machine_authority_stale: external lease refused"}});
            }
        };
        request.parameters["_machine_request_id"] = json!(request.request_id);
        let _admission = self.operation_admission.read().await;
        let mut owner = self.owner_control.subscribe();
        let mut dev_owner = self.dev_owner_control.subscribe();
        let owner_revision = *owner.borrow_and_update();
        let dev_revision = *dev_owner.borrow_and_update();
        if self.upgrading.load(std::sync::atomic::Ordering::Acquire)
            || owner_revision & 1 != 0
            || dev_revision & 1 != 0
        {
            self.external_authority.finish(&authority.lease_id);
            return json!({"error":{"code":12408,"message":"owner_in_control: wait for the owner"}});
        }
        let result = tokio::select! {
            biased;
            _ = stopped.changed() => Err(MachineError::AuthorityStale),
            _ = owner.changed() => Err(MachineError::OwnerInControl),
            _ = dev_owner.changed() => Err(MachineError::OwnerInControl),
            result = self.execute_external(request.operation, request.parameters, &authority) => result.map_err(MachineError::from),
        };
        if !self.external_authority.live(&authority.lease_id) {
            return json!({"error":{"code":12421,"message":"machine_authority_stale: external lease expired"}});
        }
        if *owner.borrow() != owner_revision || *dev_owner.borrow() != dev_revision {
            return json!({"error":{"code":12408,"message":"owner_in_control: wait for the owner"}});
        }
        match result {
            Ok(mut value) => {
                scrub_value(&mut value, &*self.redactor.lock().await);
                value
            }
            Err(error) => {
                let (code, message) = error.public();
                let mut value = json!({"error":{"code":code,"message":message}});
                if let MachineError::File(failure) = error {
                    value["error"]["reason"] = json!(failure.reason);
                }
                value
            }
        }
    }

    async fn execute_external(
        &self,
        operation: Operation,
        parameters: Value,
        authority: &ExternalAuthority,
    ) -> Result<Value> {
        match operation {
            Operation::Exec => {
                let roots = self.roots.restricted(&authority.file_roots)?;
                let gateway = self.gateway.get().context("machine gateway unavailable")?;
                let spec: nyxid_machine::gateway::Environment = serde_json::from_value(
                    parameters.get("environment").cloned().unwrap_or(json!({})),
                )?;
                let id = authority.job_id.as_deref().context("job missing")?;
                let environment = gateway.environment_external(id, &spec).await?;
                gateway.bind_external_authority(id, authority).await;
                let mut request: jobs::Exec = serde_json::from_value(parameters)?;
                let background = request.background;
                let timeout = request.timeout_secs.unwrap_or(120);
                anyhow::ensure!(
                    timeout > 0 && timeout <= 3600,
                    "external command timeout exceeds limit"
                );
                request.background = true;
                let result = self
                    .jobs
                    .start_external(request, &self.identity, &roots, &environment, authority)
                    .await;
                if result.is_err() {
                    gateway.remove_job(id).await;
                }
                let result = result?;
                if background {
                    Ok(result)
                } else {
                    self.jobs.foreground_result(id, timeout + 5).await
                }
            }
            Operation::Job => {
                let id = authority.job_id.as_deref().context("job missing")?;
                self.jobs.authorize_external(id, authority).await?;
                self.jobs
                    .result(
                        id,
                        parameters["wait_secs"].as_u64().unwrap_or(0).min(60),
                        parameters["output_offset"].as_u64().unwrap_or(0),
                        parameters["stderr_offset"].as_u64().unwrap_or(0),
                    )
                    .await
            }
            Operation::ListFiles
            | Operation::ReadFile
            | Operation::WriteFile
            | Operation::EditFile => {
                if operation == Operation::WriteFile {
                    enforce_managed_write_limit(&parameters)?;
                }
                let roots = self.roots.restricted(&authority.file_roots)?;
                self.file_operation_with_roots(operation, parameters, roots.paths().to_vec())
                    .await
            }
            _ => bail!("external operation unsupported"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nyxid_machine::authority::{Capabilities, ExternalPrincipal};

    fn authority(runtime: &Runtime, root: &Path) -> ExternalAuthority {
        let id = || uuid::Uuid::new_v4().to_string();
        let owner = id();
        let now = chrono::Utc::now().timestamp_millis();
        ExternalAuthority {
            principal: ExternalPrincipal::ApiKey {
                user_id: owner.clone(),
                api_key_id: id(),
                api_key_credential_id: Some(id()),
            },
            oauth_consent: None,
            context_id: id(),
            generation: 1,
            mode: "shared_legacy".into(),
            owner_id: owner.clone(),
            actor_id: owner,
            node_id: runtime.node_id.clone(),
            runtime_id: runtime.runtime_id.clone(),
            runtime_generation: None,
            credential_epoch: None,
            grant_id: id(),
            grant_revision: 1,
            assignment_revision: 1,
            lease_id: id(),
            job_id: Some(id()),
            expires_at_ms: now + 30_000,
            auth_expires_at_ms: now + 120_000,
            job_expires_at_ms: now + 120_000,
            funding_expires_at_ms: None,
            capabilities: Capabilities {
                shell: true,
                files: true,
                ..Default::default()
            },
            file_roots: vec![root.display().to_string()],
            services: vec![],
        }
    }
    fn signed(
        runtime: &Runtime,
        operation: Operation,
        parameters: Value,
        authority: ExternalAuthority,
    ) -> Request {
        let mut request = Request {
            version: 3,
            authority: None,
            external_authority: Some(Box::new(authority)),
            request_id: uuid::Uuid::new_v4().to_string(),
            node_id: runtime.node_id.clone(),
            operation,
            parameters,
            timestamp: chrono::Utc::now().timestamp(),
            nonce: uuid::Uuid::new_v4().to_string(),
            signature: String::new(),
        };
        request.signature = nyxid_machine::signing::sign(&request, b"external-node-test");
        request
    }
    async fn fixture() -> (
        tempfile::TempDir,
        Arc<Runtime>,
        ExternalAuthority,
        tokio::sync::mpsc::Receiver<crate::node::ws_client::NodeWsMessage>,
    ) {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("identity");
        let workspace = root.path().join("workspace");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::create_dir_all(&workspace).unwrap();
        let config = Config {
            shell: true,
            files: true,
            roots: vec![workspace.clone()],
            allow_root: true,
            max_jobs: 2,
            ..Default::default()
        };
        let runtime = Runtime::new(&config, &uuid::Uuid::new_v4().to_string(), &directory).unwrap();
        let (tx, rx) = tokio::sync::mpsc::channel(100);
        runtime.connect(tx, b"external-node-test").await.unwrap();
        let authority = authority(&runtime, &workspace);
        (root, runtime, authority, rx)
    }
    async fn start(runtime: &Arc<Runtime>, authority: &ExternalAuthority) -> Value {
        runtime.handle(signed(runtime, Operation::Exec, json!({"job_id":authority.job_id,"command":"printf before; sleep 30 & wait","background":true,"timeout_secs":30}), authority.clone()), b"external-node-test").await
    }

    fn signed_owner(runtime: &Runtime, a: &ExternalAuthority, operation: Operation) -> Request {
        let access = nyxid_machine::authority::OwnerJobAccess {
            owner_id: a.owner_id.clone(),
            node_id: runtime.node_id.clone(),
            runtime_id: runtime.runtime_id.clone(),
            job_id: a.job_id.clone().unwrap(),
            expires_at_ms: chrono::Utc::now().timestamp_millis() + 30_000,
        };
        let mut request = signed(
            runtime,
            operation,
            json!({"job_id":a.job_id,"owner_job_access":access}),
            a.clone(),
        );
        request.version = 1;
        request.external_authority = None;
        request.signature = nyxid_machine::signing::sign(&request, b"external-node-test");
        request
    }

    #[tokio::test]
    async fn owner_job_access_survives_source_revoke_without_renewing_it() {
        let (_root, runtime, a, _rx) = fixture().await;
        assert!(start(&runtime, &a).await.get("error").is_none());
        let mut wrong_owner = signed_owner(&runtime, &a, Operation::Job);
        wrong_owner.parameters["owner_job_access"]["owner_id"] =
            json!(uuid::Uuid::new_v4().to_string());
        wrong_owner.signature = nyxid_machine::signing::sign(&wrong_owner, b"external-node-test");
        assert_eq!(
            runtime.handle(wrong_owner, b"external-node-test").await["error"]["code"],
            12403
        );
        let mut stale = signed_owner(&runtime, &a, Operation::Job);
        stale.parameters["owner_job_access"]["expires_at_ms"] = json!(1);
        stale.signature = nyxid_machine::signing::sign(&stale, b"external-node-test");
        assert_eq!(
            runtime.handle(stale, b"external-node-test").await["error"]["code"],
            12421
        );
        let mut forged = signed_owner(&runtime, &a, Operation::Job);
        forged.signature = "00".repeat(32);
        assert_eq!(
            runtime.handle(forged, b"external-node-test").await["error"]["code"],
            12401
        );
        let revoked = runtime
            .handle(
                signed(&runtime, Operation::AuthorityRevoke, json!({}), a.clone()),
                b"external-node-test",
            )
            .await;
        assert_eq!(revoked["accepted"], true);
        let cancel = runtime
            .handle(
                signed_owner(&runtime, &a, Operation::JobCancel),
                b"external-node-test",
            )
            .await;
        assert_eq!(cancel["cancel_requested"], true, "{cancel}");
        assert!(cancel.get("stdout").is_none());
        let output = runtime
            .handle(
                signed_owner(&runtime, &a, Operation::Job),
                b"external-node-test",
            )
            .await;
        assert_eq!(output["job_id"], a.job_id.as_deref().unwrap(), "{output}");
        assert!(!runtime.external_authority.live(&a.lease_id));
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn legacy_v1_and_assistant_v2_cannot_read_or_cancel_external_job_ids() {
        let (_root, runtime, a, _rx) = fixture().await;
        assert!(start(&runtime, &a).await.get("error").is_none());
        for version in [1, 2] {
            for operation in [Operation::Job, Operation::JobCancel] {
                let mut request = signed_owner(&runtime, &a, operation);
                request
                    .parameters
                    .as_object_mut()
                    .unwrap()
                    .remove("owner_job_access");
                request.version = version;
                if version == 2 {
                    let id = || uuid::Uuid::new_v4().to_string();
                    let authority = nyxid_machine::authority::Authority {
                        require_v2: true,
                        context_id: id(),
                        generation: 1,
                        mode: "shared_legacy".into(),
                        agent_id: id(),
                        owner_id: a.owner_id.clone(),
                        actor_id: a.owner_id.clone(),
                        group_id: None,
                        runtime_id: runtime.runtime_id.clone(),
                        conversation_id: id(),
                        turn_id: id(),
                        lease_id: id(),
                        revision: 1,
                        expires_at_ms: chrono::Utc::now().timestamp_millis() + 30_000,
                        capabilities: Capabilities {
                            shell: true,
                            ..Default::default()
                        },
                    };
                    request.parameters["conversation_id"] = json!(authority.conversation_id);
                    request.parameters["turn_id"] = json!(authority.turn_id);
                    request.authority = Some(Box::new(authority));
                }
                request.signature = nyxid_machine::signing::sign(&request, b"external-node-test");
                let denied = runtime.handle(request, b"external-node-test").await;
                assert_eq!(
                    denied["error"]["code"], 12403,
                    "version {version}: {denied}"
                );
                assert!(denied.get("stdout").is_none());
            }
        }
        assert!(runtime.jobs.running(a.job_id.as_deref().unwrap()).await);
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn issue_1844_owner_result_keeps_verified_skew_timing_through_send() {
        let (_root, runtime, a, _rx) = fixture().await;
        assert!(start(&runtime, &a).await.get("error").is_none());
        let mut request = signed_owner(&runtime, &a, Operation::Job);
        request.timestamp += 3;
        request.parameters["owner_job_access"]["expires_at_ms"] =
            json!(chrono::Utc::now().timestamp_millis() + 48_000);
        request.signature = nyxid_machine::signing::sign(&request, b"external-node-test");
        let result = runtime.handle(request.clone(), b"external-node-test").await;
        assert!(result.get("error").is_none(), "{result}");
        let (tx, mut rx) = tokio::sync::mpsc::channel(2);
        runtime
            .send_result(
                &tx,
                &request.request_id,
                Operation::Job,
                0,
                &request.parameters,
                result,
            )
            .await;
        let crate::node::ws_client::NodeWsMessage::Text(wire) = rx.recv().await.unwrap() else {
            panic!("text")
        };
        let result: Value = serde_json::from_str(&wire).unwrap();
        assert!(result["result"].get("error").is_none(), "{result}");
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn issue_1844_external_expired_activation_retains_revision_without_work() {
        let (_root, runtime, mut a, _rx) = fixture().await;
        a.grant_revision = 2;
        let clock = nyxid_machine::authority::LeaseClock {
            now_ms: chrono::Utc::now().timestamp_millis(),
            deadline: std::time::Instant::now(),
        };
        assert!(
            runtime
                .external_authority
                .admit_at(&a, &runtime.node_id, &runtime.runtime_id, true, clock)
                .is_err()
        );
        assert!(!runtime.external_authority.live(&a.lease_id));
        assert!(runtime.external_authority.expired_jobs().is_empty());
        let reopened = external_authority::Fences::open(&runtime.status_directory).unwrap();
        a.grant_revision = 1;
        assert!(
            reopened
                .admit(&a, &runtime.node_id, &runtime.runtime_id, true)
                .is_err()
        );
        a.grant_revision = 2;
        reopened
            .admit(&a, &runtime.node_id, &runtime.runtime_id, true)
            .unwrap();
        a.expires_at_ms += 1;
        assert!(
            reopened
                .renew_at(&a, &runtime.node_id, &runtime.runtime_id, clock)
                .is_err()
        );
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn issue_1844_owner_cancel_rechecks_deadline_after_authorization() {
        let (_root, runtime, a, _rx) = fixture().await;
        assert!(start(&runtime, &a).await.get("error").is_none());
        let request = signed_owner(&runtime, &a, Operation::JobCancel);
        let access = runtime.owner_job_access(&request).unwrap();
        let result = runtime
            .handle_owner_job_at(
                request,
                access,
                nyxid_machine::authority::LeaseClock {
                    now_ms: chrono::Utc::now().timestamp_millis(),
                    deadline: std::time::Instant::now(),
                },
            )
            .await;
        assert_eq!(result["error"]["code"], 12421, "{result}");
        assert!(
            runtime.jobs.running(a.job_id.as_deref().unwrap()).await,
            "expired cancellation must have no effect"
        );
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn issue_1844_owner_cancel_receipt_replay_retains_original_deadline() {
        let (_root, runtime, a, _rx) = fixture().await;
        assert!(start(&runtime, &a).await.get("error").is_none());
        let request = signed_owner(&runtime, &a, Operation::JobCancel);
        let (sender, mut receiver) = tokio::sync::mpsc::channel(2);
        let mut first = None;
        for _ in 0..2 {
            let result = runtime.handle(request.clone(), b"external-node-test").await;
            assert_eq!(result["cancel_requested"], true, "{result}");
            runtime
                .send_result(
                    &sender,
                    &request.request_id,
                    Operation::JobCancel,
                    0,
                    &request.parameters,
                    result,
                )
                .await;
            let crate::node::ws_client::NodeWsMessage::Text(wire) = receiver.recv().await.unwrap()
            else {
                panic!("text")
            };
            let result: Value = serde_json::from_str(&wire).unwrap();
            assert_eq!(result["result"]["cancel_requested"], true, "{result}");
            let deadline = runtime.owner_job_deadlines.lock().unwrap()[&request.request_id].1;
            if let Some(original) = first {
                assert_eq!(deadline, original);
            } else {
                first = Some(deadline);
            }
        }
        runtime
            .owner_job_deadlines
            .lock()
            .unwrap()
            .get_mut(&request.request_id)
            .unwrap()
            .1 = std::time::Instant::now();
        let result = runtime.handle(request.clone(), b"external-node-test").await;
        runtime
            .send_result(
                &sender,
                &request.request_id,
                Operation::JobCancel,
                0,
                &request.parameters,
                result,
            )
            .await;
        let crate::node::ws_client::NodeWsMessage::Text(wire) = receiver.recv().await.unwrap()
        else {
            panic!("text")
        };
        let result: Value = serde_json::from_str(&wire).unwrap();
        assert_eq!(result["result"]["error"]["code"], 12421, "{result}");
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn owner_output_rechecks_management_expiry_after_socket_backpressure() {
        let (_root, runtime, a, _rx) = fixture().await;
        assert!(start(&runtime, &a).await.get("error").is_none());
        let request = signed_owner(&runtime, &a, Operation::Job);
        let output = runtime.handle(request.clone(), b"external-node-test").await;
        assert!(output.get("error").is_none());
        let parameters = request.parameters.clone();
        let request_id = request.request_id.clone();
        let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
        sender
            .send(crate::node::ws_client::NodeWsMessage::Text("held".into()))
            .await
            .unwrap();
        let target = runtime.clone();
        let send = tokio::spawn(async move {
            target
                .send_result(
                    &sender,
                    &request.request_id,
                    Operation::Job,
                    0,
                    &parameters,
                    output,
                )
                .await;
        });
        // Expire the verified monotonic deadline while output is queued.
        // No signature mutation or wall-clock sleep can mask this regression.
        runtime
            .owner_job_deadlines
            .lock()
            .unwrap()
            .get_mut(&request_id)
            .unwrap()
            .1 = std::time::Instant::now();
        receiver.recv().await.unwrap();
        send.await.unwrap();
        let crate::node::ws_client::NodeWsMessage::Text(result) = receiver.recv().await.unwrap()
        else {
            panic!("expected text result")
        };
        let result: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(result["result"]["error"]["code"], 12421);
        assert!(result["result"].get("stdout").is_none());
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn signed_dispatch_shares_parent_view_but_child_renewal_and_revoke_stay_precise() {
        let (_root, runtime, a, _rx) = fixture().await;
        let result = start(&runtime, &a).await;
        assert_eq!(result["job_id"], a.job_id.as_deref().unwrap(), "{result}");
        let mut b = a.clone();
        if let ExternalPrincipal::ApiKey {
            api_key_credential_id,
            ..
        } = &mut b.principal
        {
            *api_key_credential_id = Some(uuid::Uuid::new_v4().to_string());
        }
        b.lease_id = uuid::Uuid::new_v4().to_string();
        let view = runtime
            .handle(
                signed(
                    &runtime,
                    Operation::Job,
                    json!({"job_id":a.job_id}),
                    b.clone(),
                ),
                b"external-node-test",
            )
            .await;
        assert_eq!(view["status"], "running", "{view}");
        let mut forged_renewal = b.clone();
        forged_renewal.lease_id = a.lease_id.clone();
        forged_renewal.expires_at_ms += 1;
        let denied = runtime
            .handle(
                signed(
                    &runtime,
                    Operation::AuthorityRenew,
                    json!({}),
                    forged_renewal,
                ),
                b"external-node-test",
            )
            .await;
        assert_eq!(denied["error"]["code"], 12421);
        b.job_id = Some(uuid::Uuid::new_v4().to_string());
        b.lease_id = uuid::Uuid::new_v4().to_string();
        assert!(start(&runtime, &b).await.get("error").is_none());
        let mut revoked = a.clone();
        revoked.auth_expires_at_ms = 1;
        revoked.job_expires_at_ms = 1;
        let result = runtime
            .handle(
                signed(
                    &runtime,
                    Operation::AuthorityRevoke,
                    json!({"selector":"lease"}),
                    revoked,
                ),
                b"external-node-test",
            )
            .await;
        assert_eq!(result["accepted"], true, "{result}");
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while runtime.jobs.running(a.job_id.as_deref().unwrap()).await {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        assert!(runtime.jobs.running(b.job_id.as_deref().unwrap()).await);
        assert!(!runtime.external_authority.live(&a.lease_id));
        assert!(runtime.external_authority.live(&b.lease_id));
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn signed_external_dispatch_uses_the_same_two_command_slots_and_replay_does_not_restart()
    {
        let (_root, runtime, a, _rx) = fixture().await;
        let request = signed(
            &runtime,
            Operation::Exec,
            json!({"job_id":a.job_id,"command":"sleep 30","background":true,"timeout_secs":30}),
            a.clone(),
        );
        assert!(
            runtime
                .handle(request.clone(), b"external-node-test")
                .await
                .get("error")
                .is_none()
        );
        let replay = runtime.handle(request, b"external-node-test").await;
        assert_eq!(replay["job_id"], a.job_id.as_deref().unwrap());
        assert_eq!(replay["output_omitted_on_replay"], true);
        let mut b = a.clone();
        b.lease_id = uuid::Uuid::new_v4().to_string();
        b.job_id = Some(uuid::Uuid::new_v4().to_string());
        assert!(start(&runtime, &b).await.get("error").is_none());
        let mut c = b.clone();
        c.lease_id = uuid::Uuid::new_v4().to_string();
        c.job_id = Some(uuid::Uuid::new_v4().to_string());
        assert_eq!(start(&runtime, &c).await["error"]["code"], 12407);
        assert!(!runtime.jobs.running(c.job_id.as_deref().unwrap()).await);
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn managed_cutoff_blocks_external_cached_receipts_and_queued_output() {
        let (_root, runtime, mut a, _rx) = fixture().await;
        let id = || uuid::Uuid::new_v4().to_string();
        let binding = nyxid_machine::managed::Binding {
            node_id: runtime.node_id.clone(),
            sandbox_uid: id(),
            pod_uid: id(),
            runtime_generation: 1,
            credential_epoch: 1,
            writer_fence: id(),
            operation_id: id(),
            delivery_id: id(),
        };
        runtime.bind_managed_runtime(&binding).unwrap();
        a.runtime_generation = Some(binding.runtime_generation);
        a.credential_epoch = Some(binding.credential_epoch);
        let now = chrono::Utc::now().timestamp_millis();
        let window = nyxid_machine::managed::SignedRuntimeWindow::sign(
            nyxid_machine::managed::RuntimeWindow {
                binding,
                control_revision: 1,
                run_not_after_ms: now + 1_000,
                issued_at_ms: now,
            },
            &[7; 32],
        )
        .unwrap();
        runtime.apply_managed_window(&window, &[7; 32]).unwrap();
        let request = signed(
            &runtime,
            Operation::Exec,
            json!({"job_id":a.job_id,"command":"printf cached-external-output","background":false}),
            a.clone(),
        );
        let output = runtime.handle(request.clone(), b"external-node-test").await;
        assert_eq!(output["stdout"], "cached-external-output", "{output}");
        let replay = runtime.handle(request.clone(), b"external-node-test").await;
        assert!(replay.get("error").is_none(), "{replay}");
        assert_eq!(replay["output_omitted_on_replay"], true);
        assert!(replay.get("stdout").is_none());
        let mut parameters = request.parameters.clone();
        parameters["_signed_external_authority"] = serde_json::to_value(&a).unwrap();
        let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
        sender
            .send(crate::node::ws_client::NodeWsMessage::Text(
                "occupied".into(),
            ))
            .await
            .unwrap();
        let active = runtime.clone();
        let request_id = request.request_id.clone();
        let sending = tokio::spawn(async move {
            active
                .send_result(
                    &sender,
                    &request_id,
                    Operation::Exec,
                    0,
                    &parameters,
                    output,
                )
                .await;
        });
        tokio::task::yield_now().await;
        assert!(!sending.is_finished());
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while runtime.managed_window_live() {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let denied = runtime.handle(request, b"external-node-test").await;
        assert_eq!(denied["error"]["code"], 12421, "{denied}");
        assert!(denied.get("stdout").is_none());
        receiver.recv().await.unwrap();
        sending.await.unwrap();
        let crate::node::ws_client::NodeWsMessage::Text(message) = receiver.recv().await.unwrap()
        else {
            panic!("expected fenced external result");
        };
        let result: Value = serde_json::from_str(&message).unwrap();
        assert_eq!(result["result"]["error"]["code"], 12421, "{result}");
        assert!(!message.contains("cached-external-output"));
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn signed_roots_limit_cwd_without_changing_shared_shell_authority() {
        let (root, runtime, mut a, _rx) = fixture().await;
        let granted = root.path().join("workspace/granted");
        std::fs::create_dir(&granted).unwrap();
        a.file_roots = vec![granted.display().to_string()];
        let result = runtime.handle(signed(&runtime, Operation::Exec, json!({"job_id":a.job_id,"command":"true","cwd":root.path().join("workspace").display().to_string()}), a), b"external-node-test").await;
        assert_eq!(result["error"]["code"], 12402, "{result}");
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn durable_context_and_revocation_fences_survive_restart_and_reject_rebinding() {
        let (_root, runtime, a, _rx) = fixture().await;
        runtime
            .external_authority
            .admit(&a, &runtime.node_id, &runtime.runtime_id, true)
            .unwrap();
        let mut rebound = a.clone();
        rebound.lease_id = uuid::Uuid::new_v4().to_string();
        rebound.grant_id = uuid::Uuid::new_v4().to_string();
        assert!(
            runtime
                .external_authority
                .admit(&rebound, &runtime.node_id, &runtime.runtime_id, false)
                .is_err()
        );
        runtime
            .external_authority
            .revoke(&a, &runtime.node_id, &runtime.runtime_id)
            .unwrap();
        let reopened = external_authority::Fences::open(&runtime.status_directory).unwrap();
        assert!(
            reopened
                .admit(&a, &runtime.node_id, &runtime.runtime_id, true)
                .is_err()
        );
        let mut successor = a.clone();
        successor.lease_id = uuid::Uuid::new_v4().to_string();
        successor.grant_revision += 1;
        reopened
            .admit(&successor, &runtime.node_id, &runtime.runtime_id, true)
            .unwrap();
        let mut older = a.clone();
        older.lease_id = uuid::Uuid::new_v4().to_string();
        assert!(
            reopened
                .admit(&older, &runtime.node_id, &runtime.runtime_id, true)
                .is_err()
        );
        runtime.shutdown().await;
    }
}
