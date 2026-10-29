//! Context routing is selected exclusively by authenticated v2 authority. A
//! provisioning/probe failure refuses the request; shared execution is never a
//! recovery path. Parent jobs/gateway/authority fences remain node-wide.
use super::{
    Runtime,
    context_runtime::{Store, protect_legacy_browser_home, protect_legacy_roots},
    cua, files,
};
use anyhow::{Result, ensure};
use nyxid_machine::authority::Authority;
use std::{collections::HashMap, fmt, sync::Arc, time::Instant};

/// Secret-free context preparation metadata. The underlying anyhow chain is
/// intentionally not exposed to the caller; only these fixed labels may cross
/// the machine diagnostic boundary.
#[derive(Debug, Clone)]
pub(super) struct PreparationFailure {
    pub(super) stage: &'static str,
    pub(super) reason: &'static str,
    pub(super) elapsed_ms: u64,
    pub(super) correlation_id: String,
    pub(super) generation: u64,
}

impl PreparationFailure {
    pub(super) fn new(
        stage: &'static str,
        reason: &'static str,
        elapsed_ms: u64,
        correlation_id: &str,
        generation: u64,
    ) -> Self {
        Self {
            stage,
            reason,
            elapsed_ms,
            correlation_id: correlation_id.to_owned(),
            generation,
        }
    }
}

impl fmt::Display for PreparationFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.reason)
    }
}

impl std::error::Error for PreparationFailure {}

pub(super) fn stage_reason(stage: &str) -> &'static str {
    match stage {
        "authority" => "context_authority_invalid",
        "binding" => "hosted_context_binding_required",
        "identity" => "context_identity_mismatch",
        "generation" => "context_generation_quarantined",
        "capability" => "context_backend_unavailable",
        "custody" => "node_custody_unverified",
        "legacy_roots" => "legacy_roots_unavailable",
        "legacy_browser_home" => "legacy_browser_home_unavailable",
        "allocation_store" => "context_store_unavailable",
        "provision_users" => "context_user_provision_failed",
        "reap_secure" => "context_secure_reap_failed",
        "reap_developer" => "context_developer_reap_failed",
        "workspace_probe" => "workspace_probe_failed",
        "native_policy" => "native_browser_policy_failed",
        "browser_resources" => "browser_resources_unavailable",
        "context_runtime" => "context_runtime_unavailable",
        "context_connect" => "context_node_offline",
        _ => "context_preparation_failed",
    }
}

fn support_failure_stage(reason: Option<&str>) -> &'static str {
    match reason {
        Some("node_custody_unverified") => "custody",
        _ => "capability",
    }
}

pub(super) fn failure_diagnostic(error: &anyhow::Error) -> Option<&PreparationFailure> {
    error.downcast_ref::<PreparationFailure>()
}

pub(super) fn failure_stage(error: &anyhow::Error) -> &'static str {
    failure_diagnostic(error).map_or("context_preparation", |failure| failure.stage)
}

pub(super) fn failure_reason(error: &anyhow::Error) -> &'static str {
    failure_diagnostic(error).map_or("context_preparation_failed", |failure| failure.reason)
}

fn annotate<T>(
    result: Result<T>,
    stage: &'static str,
    started: Instant,
    authority: &Authority,
) -> Result<T> {
    let elapsed_ms = started.elapsed().as_millis().min(u64::MAX as u128) as u64;
    tracing::debug!(
        stage,
        reason = if result.is_ok() { "stage_ready" } else { stage_reason(stage) },
        elapsed_ms,
        correlation_id = %authority.context_id,
        generation = authority.generation,
        initialized = result.is_ok(),
        "context preparation stage"
    );
    result.map_err(|error| {
        error.context(PreparationFailure::new(
            stage,
            stage_reason(stage),
            elapsed_ms,
            &authority.context_id,
            authority.generation,
        ))
    })
}

fn preparation_error(
    stage: &'static str,
    authority: &Authority,
    started: Instant,
) -> anyhow::Error {
    anyhow::Error::new(PreparationFailure::new(
        stage,
        stage_reason(stage),
        started.elapsed().as_millis().min(u64::MAX as u128) as u64,
        &authority.context_id,
        authority.generation,
    ))
}

#[derive(Default)]
pub(super) struct Contexts {
    pub(super) store: Option<Store>,
    pub runtimes: HashMap<String, Arc<Runtime>>,
    // A prerequisite fixture for dispatch-gate tests without Chromium. Never
    // compiled into a daemon or used as real hosted/browser acceptance evidence.
    #[cfg(test)]
    pub(super) test_evidence: Option<nyxid_machine::context::Evidence>,
}

#[cfg(test)]
#[path = "context_dispatch_tests.rs"]
mod tests;
impl Runtime {
    pub(super) async fn context_children(&self) -> Vec<Arc<Runtime>> {
        self.contexts
            .lock()
            .await
            .runtimes
            .values()
            .cloned()
            .collect()
    }
    pub(super) async fn result_context(
        &self,
        operation: nyxid_machine::Operation,
        parameters: &serde_json::Value,
    ) -> Option<Arc<Runtime>> {
        use nyxid_machine::Operation;
        let authority = &parameters["_signed_authority"];
        let id = if !authority.is_null() {
            // A context ID can survive an explicit opt-out. Shared work must
            // observe the legacy display even while an old child is retiring.
            if authority["mode"] != "separated" {
                return None;
            }
            authority["context_id"].as_str()
        } else if matches!(
            operation,
            Operation::DesktopOpen
                | Operation::DesktopClose
                | Operation::DesktopControl
                | Operation::DesktopInput
        ) {
            parameters["context_id"].as_str()
        } else {
            None
        }?;
        self.contexts.lock().await.runtimes.get(id).cloned()
    }
    pub(super) async fn quarantine_profiles(
        &self,
        agent: &str,
        before_revision: i64,
    ) -> Result<()> {
        let mut contexts = self.contexts.lock().await;
        // A missing ephemeral native journal cannot prove a managed context
        // never existed. Only the original protected store can acknowledge its
        // persisted quarantine, including after a Pod/daemon replacement.
        ensure!(
            self.managed_binding.get().is_none()
                || contexts.store.as_ref().is_some_and(Store::is_hosted),
            "hosted_context_binding_required"
        );
        if contexts.store.is_none() && self.status_directory.join("machine-contexts.json").exists()
        {
            contexts.store = Some(Store::open(&self.status_directory, &self.node_id)?);
        }
        let users = match contexts.store.as_mut() {
            Some(store) => store.quarantine(agent, before_revision)?,
            None => Vec::new(),
        };
        let ids: Vec<_> = contexts
            .runtimes
            .iter()
            .filter(|(_, r)| {
                r.context_binding
                    .as_ref()
                    .is_some_and(|a| a.agent_id == agent && a.revision < before_revision)
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            if let Some(runtime) = contexts.runtimes.remove(&id) {
                if let Some(driver) = &runtime.driver {
                    driver.stop().await;
                }
                if let Some(browser) = runtime.browser.lock().await.take() {
                    browser.stop().await;
                }
                runtime.dev_browser.lock().await.take();
                runtime.desktop.session.lock().await.take();
                runtime.dev_desktop.session.lock().await.take();
            }
        }
        for uid in users {
            self.kill_context_uid(uid).await?;
        }
        Ok(())
    }
    async fn kill_context_uid(&self, uid: u32) -> Result<()> {
        ensure!(
            uid >= nyxid_machine::context::FIRST_UID,
            "context_user_invalid"
        );
        // Journaled numeric identity also works after a container recreation
        // removed /etc/passwd entries. No external account lookup or UID reuse.
        let mut identity = self.identity.clone();
        identity.uid = uid;
        identity.gid = uid;
        identity.policy_groups.clear();
        identity.sandbox = None;
        identity.desktop = None;
        let mut command = tokio::process::Command::new("/bin/true");
        identity.prepare(&mut command)?;
        unsafe {
            command.pre_exec(|| {
                // kill(-1) excludes this helper; DAC restricts it to this UID.
                if libc::kill(-1, libc::SIGKILL) < 0
                    && !matches!(
                        std::io::Error::last_os_error().raw_os_error(),
                        Some(libc::ESRCH | libc::EPERM)
                    )
                {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        ensure!(
            command.status().await?.success(),
            "context_quarantine_failed"
        );
        Ok(())
    }
    pub(super) async fn context_instance(&self, authority: &Authority) -> Result<Arc<Runtime>> {
        let started = Instant::now();
        if authority.mode != "separated" || !authority.require_v2 {
            return Err(preparation_error("authority", authority, started));
        }
        let mut contexts = self.contexts.lock().await;
        if self.managed_binding.get().is_some()
            && !contexts.store.as_ref().is_some_and(Store::is_hosted)
        {
            return Err(preparation_error("binding", authority, started));
        }
        if let Some(runtime) = contexts.runtimes.get(&authority.context_id) {
            let binding = runtime
                .context_binding
                .as_ref()
                .ok_or_else(|| preparation_error("binding", authority, Instant::now()))?;
            if nyxid_machine::context::Binding::from(binding)
                != nyxid_machine::context::Binding::from(authority)
            {
                return Err(preparation_error("identity", authority, Instant::now()));
            }
            if authority.generation < binding.generation {
                return Err(preparation_error("generation", authority, Instant::now()));
            }
            if authority.generation == binding.generation {
                return Ok(runtime.clone());
            }
            // Cancel the old runtime and drop every browser/display before a new
            // profile generation is opened. Old directory/UID tombstones remain.
            // Parent authority admission already cancels the old revision.
            // Do not stop unrelated agents sharing the node-wide turn registry.
            if let Some(driver) = &runtime.driver {
                driver.stop().await;
            }
            if let Some(browser) = runtime.browser.lock().await.take() {
                browser.stop().await;
            }
            runtime.dev_browser.lock().await.take();
            runtime.desktop.session.lock().await.take();
            runtime.dev_desktop.session.lock().await.take();
            let stage_started = Instant::now();
            annotate(
                self.kill_context_uid(runtime.browser_identity.uid).await,
                "reap_secure",
                stage_started,
                authority,
            )?;
            if let Some(dev) = &runtime.dev_identity {
                let stage_started = Instant::now();
                annotate(
                    self.kill_context_uid(dev.uid).await,
                    "reap_developer",
                    stage_started,
                    authority,
                )?;
            }
            contexts.runtimes.remove(&authority.context_id);
        }
        let support_started = Instant::now();
        #[cfg(test)]
        let support = match &contexts.test_evidence {
            Some(evidence) => evidence.support(),
            None => self.context_support().await,
        };
        #[cfg(not(test))]
        let support = self.context_support().await;
        if !support.available {
            return Err(preparation_error(
                support_failure_stage(support.reason.as_deref()),
                authority,
                support_started,
            ));
        }
        annotate(Ok(()), "capability", support_started, authority)?;
        let directory = self.status_directory.clone();
        let node = self.node_id.clone();
        let mut roots = self.config.roots.clone();
        if !roots.contains(&self.identity.home) {
            roots.push(self.identity.home.clone());
        }
        let legacy = self.identity.clone();
        let browsers = [
            Some(self.browser_identity.clone()),
            self.dev_identity.clone(),
        ];
        let a = authority.clone();
        let store = contexts.store.take();
        let (store, prepared) = tokio::task::spawn_blocking(move || -> Result<_> {
            let stage_started = Instant::now();
            annotate(
                protect_legacy_roots(&roots, &legacy),
                "legacy_roots",
                stage_started,
                &a,
            )?;
            for browser in browsers.into_iter().flatten() {
                let stage_started = Instant::now();
                annotate(
                    protect_legacy_browser_home(&browser),
                    "legacy_browser_home",
                    stage_started,
                    &a,
                )?;
            }
            let mut store = match store {
                Some(s) => s,
                None => {
                    let stage_started = Instant::now();
                    annotate(
                        Store::open(&directory, &node),
                        "allocation_store",
                        stage_started,
                        &a,
                    )?
                }
            };
            let stage_started = Instant::now();
            let prepared = annotate(store.provision(&a), "provision_users", stage_started, &a);
            Ok((store, prepared))
        })
        .await
        .map_err(|_| preparation_error("provision_users", authority, started))??;
        contexts.store = Some(store);
        let mut context = prepared?;
        // Reap survivors from a daemon restart before opening the same durable
        // generation. These UIDs are exclusively owned by this context journal.
        let stage_started = Instant::now();
        annotate(
            self.kill_context_uid(context.secure.uid).await,
            "reap_secure",
            stage_started,
            authority,
        )?;
        let stage_started = Instant::now();
        annotate(
            self.kill_context_uid(context.dev.uid).await,
            "reap_developer",
            stage_started,
            authority,
        )?;
        let sandbox = context
            .command
            .sandbox
            .as_ref()
            .ok_or_else(|| preparation_error("workspace_probe", authority, Instant::now()))?;
        let probe_started = Instant::now();
        annotate(
            sandbox.probe(&context.command).await,
            "workspace_probe",
            probe_started,
            authority,
        )?;
        let dev_policy = self
            .dev_identity
            .as_ref()
            .ok_or_else(|| preparation_error("browser_resources", authority, Instant::now()))?;
        if !self
            .config
            .managed_browser
            .as_ref()
            .is_some_and(|c| c.container)
        {
            let managed_browser = self
                .config
                .managed_browser
                .as_ref()
                .ok_or_else(|| preparation_error("native_policy", authority, Instant::now()))?;
            let stage_started = Instant::now();
            let layout = annotate(
                super::browser_layout::Layout::detect(&managed_browser.binary),
                "native_policy",
                stage_started,
                authority,
            )?;
            let stage_started = Instant::now();
            annotate(
                super::context_policy::prepare(std::path::Path::new("/"), layout, context.dev.uid),
                "native_policy",
                stage_started,
                authority,
            )?;
        }
        let stage_started = Instant::now();
        let resources = annotate(
            context
                .browsers(self.browser_identity.gid, dev_policy.gid)
                .await,
            "browser_resources",
            stage_started,
            authority,
        )?;
        let mut config = self.config.clone();
        config.agent_user = Some(context.command.name.clone());
        config.browser_user = Some(context.secure.name.clone());
        config.dev_browser_user = Some(context.dev.name.clone());
        config.roots = vec![context.workspace.clone()];
        let managed = config
            .managed_browser
            .as_mut()
            .ok_or_else(|| preparation_error("context_runtime", authority, Instant::now()))?;
        managed.data_dir = context
            .secure
            .home
            .parent()
            .ok_or_else(|| preparation_error("context_runtime", authority, Instant::now()))?
            .to_owned();
        let state = context
            .browser_root
            .parent()
            .ok_or_else(|| preparation_error("context_runtime", authority, Instant::now()))?
            .join("node");
        let stage_started = Instant::now();
        annotate(
            super::browser::runtime_directory(&state, 0, 0, 0o700),
            "context_runtime",
            stage_started,
            authority,
        )?;
        let stage_started = Instant::now();
        let mut runtime = annotate(
            Runtime::new(&config, &self.node_id, &state),
            "context_runtime",
            stage_started,
            authority,
        )?;
        let target = Arc::get_mut(&mut runtime)
            .ok_or_else(|| preparation_error("context_runtime", authority, Instant::now()))?;
        target.custody = Arc::new(self.custody.for_context());
        target.runtime_id = self.runtime_id.clone();
        target.identity = context.command;
        target.browser_identity = context.secure;
        target.dev_identity = Some(context.dev);
        let stage_started = Instant::now();
        target.roots = annotate(
            files::Roots::new(&config.roots, self.roots.exclusions()),
            "context_runtime",
            stage_started,
            authority,
        )?;
        target.jobs = self.jobs.clone();
        target.operation_receipts = self.operation_receipts.clone();
        target.upgrading = self.upgrading.clone();
        target.operation_admission = self.operation_admission.clone();
        target.turns = self.turns.clone();
        target.redactor = self.redactor.clone();
        target.context_resources = Some(resources);
        target.context_binding = Some(authority.clone());
        target.driver = config.cua_driver.as_ref().map(|p| {
            cua::Driver::new(
                p.clone(),
                target.browser_identity.clone(),
                config.computer_mode,
            )
        });
        target.owner_driver = config.cua_driver.as_ref().map(|p| {
            cua::Driver::new(
                p.clone(),
                target.browser_identity.clone(),
                config.computer_mode,
            )
            .for_human()
        });
        if let Some(gateway) = self.gateway.get() {
            let _ = target.gateway.set(gateway.clone());
        }
        let stage_started = Instant::now();
        target.refresh_custody().await;
        annotate(
            target.require_custody(),
            "custody",
            stage_started,
            authority,
        )?;
        // Child runtime cannot create a nested context; only this parent routes.
        let sender = self
            .desktop
            .sender
            .lock()
            .await
            .clone()
            .ok_or_else(|| preparation_error("context_connect", authority, Instant::now()))?;
        let secret = self.desktop_secret.lock().await;
        let secret = secret
            .as_deref()
            .ok_or_else(|| preparation_error("context_connect", authority, Instant::now()))?;
        let stage_started = Instant::now();
        annotate(
            runtime.connect(sender, secret).await,
            "context_connect",
            stage_started,
            authority,
        )?;
        contexts
            .runtimes
            .insert(authority.context_id.clone(), runtime.clone());
        Ok(runtime)
    }
}
