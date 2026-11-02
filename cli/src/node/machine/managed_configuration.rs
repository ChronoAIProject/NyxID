//! Hosted configuration installation is independent of Agent authority ACK.
use super::*;
use nyxid_machine::managed::{Binding, ConfigurationReport};

impl Runtime {
    /// Called on the parent only after the authenticated socket acknowledged
    /// the exact sealed managed binding. Neither Agent authority nor its ACK
    /// can populate this report.
    pub fn bind_managed_configuration(
        &self,
        binding: &Binding,
        report: &ConfigurationReport,
    ) -> Result<()> {
        crate::node::managed_config::validate_configuration_report(&self.config, binding, report)?;
        ensure!(binding.node_id == self.node_id, "managed Node mismatch");
        if let Some(current) = self.managed_configuration.get() {
            ensure!(current == report, "managed installed configuration changed");
        }
        self.bind_managed_runtime(binding)?;
        if self.managed_configuration.get().is_none() {
            self.managed_configuration
                .set(report.clone())
                .map_err(|_| anyhow::anyhow!("managed configuration already bound"))?;
        }
        Ok(())
    }

    pub(super) fn managed_configuration_report(&self) -> Option<ConfigurationReport> {
        let binding = self.managed_binding.get()?;
        let report = self.managed_configuration.get()?;
        crate::node::managed_config::validate_configuration_report(&self.config, binding, report)
            .ok()?;
        Some(report.clone())
    }

    /// Production WSS calls this only after matching its current storage grant
    /// to the sealed allocation and the parent daemon's live runtime UUID.
    /// Never call this with environment/path-existence inference. In particular
    /// runtime generation 1 and credential epoch 1 do not authorize allocation.
    #[cfg(target_os = "linux")]
    pub(crate) async fn install_hosted_context_store(
        &self,
        original_pvc_uid: &str,
        fresh_allocation: bool,
    ) -> Result<()> {
        use super::context_runtime::Store;
        let binding = self
            .managed_binding
            .get()
            .context("hosted_context_binding_required")?;
        ensure!(
            self.context_binding.is_none() && self.managed_configuration_report().is_some(),
            "hosted_context_parent_required"
        );
        let volume = super::hosted_context::VolumeBinding {
            node_id: binding.node_id.clone(),
            pvc_uid: original_pvc_uid.into(),
        };
        let mut contexts = self.contexts.lock().await;
        if let Some(store) = &contexts.store {
            ensure!(
                store.hosted_volume_matches(&volume),
                "hosted_context_volume_changed"
            );
            return Ok(());
        }
        let directory = self.status_directory.clone();
        let runtime_id = self.runtime_id.clone();
        let store = tokio::task::spawn_blocking(move || {
            Store::open_hosted(&directory, volume, &runtime_id, fresh_allocation)
        })
        .await??;
        contexts.store = Some(store);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Binding, ConfigurationReport) {
        let id = || uuid::Uuid::new_v4().to_string();
        let binding = Binding {
            node_id: id(),
            sandbox_uid: id(),
            pod_uid: id(),
            runtime_generation: 3,
            credential_epoch: 4,
            writer_fence: id(),
            operation_id: id(),
            delivery_id: id(),
        };
        let configuration = nyxid_machine::managed::RequiredConfiguration {
            version: nyxid_machine::managed::CONFIG_VERSION,
            revision: 9,
            separation_choice: nyxid_machine::context::Choice::RequireSeparated,
            capabilities: vec!["files".into(), "shell".into()],
        };
        let report = ConfigurationReport {
            digest: configuration.digest().unwrap(),
            configuration,
            runtime_generation: binding.runtime_generation,
        };
        (binding, report)
    }

    #[tokio::test]
    async fn managed_report_cannot_claim_a_configuration_that_the_daemon_did_not_install() {
        let dir = tempfile::tempdir().unwrap();
        let (binding, report) = fixture();
        let runtime = Runtime::new(&Config::default(), &binding.node_id, dir.path()).unwrap();
        runtime.bind_managed_runtime(&binding).unwrap();
        assert!(
            runtime
                .bind_managed_configuration(&binding, &report)
                .is_err()
        );
        assert!(runtime.managed_configuration_report().is_none());
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn managed_quarantine_without_the_retained_store_never_acknowledges_cleanup() {
        let directory = tempfile::tempdir().unwrap();
        let (binding, _) = fixture();
        let runtime = Runtime::new(&Config::default(), &binding.node_id, directory.path()).unwrap();
        runtime.bind_managed_runtime(&binding).unwrap();
        let id = || uuid::Uuid::new_v4().to_string();
        let authority = nyxid_machine::authority::Authority {
            require_v2: true,
            context_id: id(),
            generation: 1,
            mode: "separated".into(),
            agent_id: id(),
            owner_id: id(),
            actor_id: id(),
            group_id: None,
            runtime_id: runtime.runtime_id.clone(),
            conversation_id: id(),
            turn_id: id(),
            lease_id: id(),
            revision: 2,
            expires_at_ms: chrono::Utc::now().timestamp_millis() + 20_000,
            capabilities: Default::default(),
        };
        let mut request = Request {
            version: 2,
            authority: Some(Box::new(authority.clone())),
            external_authority: None,
            request_id: id(),
            node_id: binding.node_id,
            operation: Operation::AuthorityRevoke,
            parameters: json!({"quarantine_profiles":true}),
            timestamp: chrono::Utc::now().timestamp(),
            nonce: id(),
            signature: String::new(),
        };
        request.signature = nyxid_machine::signing::sign(&request, b"fixture-only");
        let result = runtime.handle(request, b"fixture-only").await;
        assert_eq!(result["error"]["reason"], "context_quarantine_pending");
        assert!(result.get("accepted").is_none());
        assert!(runtime.contexts.lock().await.store.is_none());
        assert!(!directory.path().join("machine-contexts.json").exists());
        let restarted =
            Runtime::new(&Config::default(), &runtime.node_id, directory.path()).unwrap();
        assert!(
            restarted.authority.enrolled(),
            "revision fence must remain durable even while cleanup is pending"
        );
        assert!(
            restarted
                .authority
                .admit(&authority, &restarted.runtime_id, None)
                .is_err()
        );
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    #[ignore = "requires the disposable hosted Linux accounts, retained tmpfs and root supervisor"]
    async fn hosted_parent_binds_report_and_store_and_rejects_unbound_quarantine() {
        use std::os::unix::fs::PermissionsExt;
        let parent = tempfile::tempdir_in("/var/lib/nyxid-hosted-tests").unwrap();
        std::fs::set_permissions(parent.path(), std::fs::Permissions::from_mode(0o711)).unwrap();
        let directory = parent.path().join("node");
        std::fs::create_dir(&directory).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        let (binding, report) = fixture();
        let config =
            crate::node::managed_config::configuration_machine(&report.configuration).unwrap();
        let runtime = Runtime::new(&config, &binding.node_id, &directory).unwrap();
        assert!(runtime.managed_configuration_report().is_none());
        runtime
            .bind_managed_configuration(&binding, &report)
            .unwrap();
        assert_eq!(runtime.managed_configuration_report(), Some(report.clone()));
        assert!(
            runtime
                .quarantine_profiles(&uuid::Uuid::new_v4().to_string(), 2)
                .await
                .is_err()
        );
        assert!(runtime.contexts.lock().await.store.is_none());
        let pvc = uuid::Uuid::new_v4().to_string();
        runtime
            .install_hosted_context_store(&pvc, true)
            .await
            .unwrap();
        runtime
            .install_hosted_context_store(&pvc, true)
            .await
            .unwrap();
        assert!(
            runtime
                .install_hosted_context_store(&uuid::Uuid::new_v4().to_string(), false)
                .await
                .is_err()
        );
        assert!(
            runtime
                .contexts
                .lock()
                .await
                .store
                .as_ref()
                .unwrap()
                .is_hosted()
        );
        assert!(
            runtime
                .quarantine_profiles(&uuid::Uuid::new_v4().to_string(), 2)
                .await
                .is_ok()
        );
        let id = || uuid::Uuid::new_v4().to_string();
        let mut authority = nyxid_machine::authority::Authority {
            require_v2: true,
            context_id: id(),
            generation: 1,
            mode: "separated".into(),
            agent_id: id(),
            owner_id: id(),
            actor_id: id(),
            group_id: None,
            runtime_id: runtime.runtime_id.clone(),
            conversation_id: id(),
            turn_id: id(),
            lease_id: id(),
            revision: 1,
            expires_at_ms: 1,
            capabilities: Default::default(),
        };
        let first = runtime
            .contexts
            .lock()
            .await
            .store
            .as_mut()
            .unwrap()
            .provision(&authority)
            .unwrap();
        let uid = first.command.uid;
        let workspace = first.workspace.clone();
        let marker = workspace.join("marker");
        let status = std::process::Command::new("setpriv")
            .args([
                "--reuid",
                &uid.to_string(),
                "--regid",
                &uid.to_string(),
                "--clear-groups",
                "sh",
                "-c",
                "printf retained > \"$1\"",
                "sh",
            ])
            .arg(&marker)
            .status()
            .unwrap();
        assert!(status.success());
        let profile = runtime.profile().await;
        assert_eq!(profile.managed_configuration, Some(report.clone()));
        assert_eq!(profile.runtime_id, runtime.runtime_id);
        assert!(!profile.browser_enabled() && !profile.computer);
        assert_eq!(
            profile.separation_evidence.unwrap().version,
            nyxid_machine::context::EVIDENCE_VERSION
        );
        let mut changed = report.clone();
        changed.runtime_generation += 1;
        assert!(
            runtime
                .bind_managed_configuration(&binding, &changed)
                .is_err()
        );
        changed = report.clone();
        changed.configuration.revision += 1;
        changed.digest = changed.configuration.digest().unwrap();
        assert!(
            runtime
                .bind_managed_configuration(&binding, &changed)
                .is_err()
        );
        let previous_runtime = runtime.runtime_id.clone();
        drop(first);
        drop(runtime);
        let restarted = Runtime::new(&config, &binding.node_id, &directory).unwrap();
        assert_ne!(previous_runtime, restarted.runtime_id);
        restarted
            .bind_managed_configuration(&binding, &report)
            .unwrap();
        restarted
            .install_hosted_context_store(&pvc, false)
            .await
            .unwrap();
        assert!(
            restarted
                .contexts
                .lock()
                .await
                .store
                .as_mut()
                .unwrap()
                .provision(&authority)
                .is_err()
        );
        // Only an authority generation supplied by the existing caller contract
        // can reconstruct profiles; the daemon never increments it itself.
        authority.generation = 2;
        authority.revision = 2;
        authority.runtime_id = restarted.runtime_id.clone();
        authority.lease_id = id();
        let next = restarted
            .contexts
            .lock()
            .await
            .store
            .as_mut()
            .unwrap()
            .provision(&authority)
            .unwrap();
        assert_eq!(next.workspace, workspace);
        assert_eq!(next.command.uid, uid);
        let status = std::process::Command::new("setpriv")
            .args([
                "--reuid",
                &uid.to_string(),
                "--regid",
                &uid.to_string(),
                "--clear-groups",
                "test",
                "-f",
            ])
            .arg(marker)
            .status()
            .unwrap();
        assert!(status.success());
        assert_ne!(next.secure.uid, 0);
        let journal: Value = serde_json::from_slice(
            &std::fs::read("/var/lib/nyxid-contexts/metadata/journal.json").unwrap(),
        )
        .unwrap();
        assert_eq!(
            journal["workspaces"][&authority.context_id]["runtime_id"],
            restarted.runtime_id
        );
        restarted
            .quarantine_profiles(&authority.agent_id, 3)
            .await
            .unwrap();
        assert!(
            restarted
                .contexts
                .lock()
                .await
                .store
                .as_mut()
                .unwrap()
                .provision(&authority)
                .is_err()
        );
    }
}
