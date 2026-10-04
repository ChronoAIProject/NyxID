//! Context routing is selected exclusively by authenticated v2 authority. A
//! provisioning/probe failure refuses the request; shared execution is never a
//! recovery path. Parent jobs/gateway/authority fences remain node-wide.
use super::{
    Runtime,
    context_runtime::{Store, protect_legacy_roots},
    cua, files, process,
};
use anyhow::{Context, Result, ensure};
use nyxid_machine::{authority::Authority, context::Support};
use std::{collections::HashMap, sync::Arc};

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub(super) struct Stage(pub &'static str);

pub(super) fn failure_stage(error: &anyhow::Error) -> &'static str {
    error
        .downcast_ref::<Stage>()
        .map_or("context_preparation", |stage| stage.0)
}

#[derive(Default)]
pub(super) struct Contexts {
    store: Option<Store>,
    pub runtimes: HashMap<String, Arc<Runtime>>,
}
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
    pub(super) async fn context_support(&self) -> Support {
        let abi = process::context_sandbox::abi()
            .ok()
            .and_then(|a| u32::try_from(a).ok());
        let reason = if abi.is_none_or(|a| a < 6) {
            Some("separated_requires_landlock_abi_6")
        } else if unsafe { libc::geteuid() } != 0
            || self.identity.uid == 0
            || self.identity.gid == 0
            || self.browser_identity.uid == 0
            || self.browser_identity.gid == 0
            || self.identity.uid == self.browser_identity.uid
            || self.identity.gid == self.browser_identity.gid
            || self.dev_identity.as_ref().is_none_or(|d| {
                d.uid == 0
                    || d.gid == 0
                    || d.uid == self.identity.uid
                    || d.uid == self.browser_identity.uid
                    || d.gid == self.identity.gid
                    || d.gid == self.browser_identity.gid
            })
        {
            Some("separated_requires_separate_users")
        } else if self.config.managed_browser.is_none() {
            Some("separated_requires_managed_browser")
        } else if process::context_sandbox::probe(&self.identity)
            .await
            .is_err()
        {
            Some("separated_landlock_probe_failed")
        } else {
            None
        };
        Support {
            available: reason.is_none(),
            landlock_abi: abi,
            reason: reason.map(str::to_owned),
        }
    }
    pub(super) async fn quarantine_profiles(
        &self,
        agent: &str,
        before_revision: i64,
    ) -> Result<()> {
        let mut contexts = self.contexts.lock().await;
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
        ensure!(
            authority.mode == "separated" && authority.require_v2,
            "context_authority_invalid"
        );
        let mut contexts = self.contexts.lock().await;
        if let Some(runtime) = contexts.runtimes.get(&authority.context_id) {
            let binding = runtime
                .context_binding
                .as_ref()
                .context("context_binding_missing")?;
            ensure!(
                nyxid_machine::context::Binding::from(binding)
                    == nyxid_machine::context::Binding::from(authority),
                "context_identity_mismatch"
            );
            ensure!(
                authority.generation >= binding.generation,
                "context_generation_quarantined"
            );
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
            self.kill_context_uid(runtime.browser_identity.uid).await?;
            if let Some(dev) = &runtime.dev_identity {
                self.kill_context_uid(dev.uid).await?;
            }
            contexts.runtimes.remove(&authority.context_id);
        }
        ensure!(
            self.context_support().await.available,
            "context_backend_unavailable"
        );
        let directory = self.status_directory.clone();
        let node = self.node_id.clone();
        let mut roots = self.config.roots.clone();
        if !roots.contains(&self.identity.home) {
            roots.push(self.identity.home.clone());
        }
        let legacy = self.identity.clone();
        let a = authority.clone();
        let store = contexts.store.take();
        let (store, prepared) = tokio::task::spawn_blocking(move || -> Result<_> {
            protect_legacy_roots(&roots, &legacy).context(Stage("legacy_roots"))?;
            let mut store = match store {
                Some(s) => s,
                None => Store::open(&directory, &node).context(Stage("allocation_store"))?,
            };
            let result = store.provision(&a).context(Stage("provision_users"));
            Ok((store, result))
        })
        .await??;
        contexts.store = Some(store);
        let mut context = prepared?;
        // Reap survivors from a daemon restart before opening the same durable
        // generation. These UIDs are exclusively owned by this context journal.
        self.kill_context_uid(context.secure.uid)
            .await
            .context(Stage("reap_secure"))?;
        self.kill_context_uid(context.dev.uid)
            .await
            .context(Stage("reap_developer"))?;
        context
            .command
            .sandbox
            .as_ref()
            .context("context_sandbox_missing")?
            .probe(&context.command)
            .await
            .context(Stage("workspace_probe"))?;
        let dev_policy = self
            .dev_identity
            .as_ref()
            .context("context_dev_user_missing")?;
        if !self
            .config
            .managed_browser
            .as_ref()
            .is_some_and(|c| c.container)
        {
            super::context_policy::prepare(std::path::Path::new("/"), context.dev.uid)
                .context(Stage("native_policy"))?;
        }
        let resources = context
            .browsers(self.browser_identity.gid, dev_policy.gid)
            .await
            .context(Stage("browser_resources"))?;
        let mut config = self.config.clone();
        config.agent_user = Some(context.command.name.clone());
        config.browser_user = Some(context.secure.name.clone());
        config.dev_browser_user = Some(context.dev.name.clone());
        config.roots = vec![context.workspace.clone()];
        let managed = config
            .managed_browser
            .as_mut()
            .context("context_browser_missing")?;
        managed.data_dir = context
            .secure
            .home
            .parent()
            .context("context_role_missing")?
            .to_owned();
        let state = context
            .browser_root
            .parent()
            .context("context_root_missing")?
            .join("node");
        super::browser::runtime_directory(&state, 0, 0, 0o700)?;
        let mut runtime =
            Runtime::new(&config, &self.node_id, &state).context(Stage("context_runtime"))?;
        let target = Arc::get_mut(&mut runtime).context("context_runtime_unavailable")?;
        target.runtime_id = self.runtime_id.clone();
        target.identity = context.command;
        target.browser_identity = context.secure;
        target.dev_identity = Some(context.dev);
        target.roots = files::Roots::new(&config.roots, &self.excluded)?;
        target.jobs = self.jobs.clone();
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
        // Child runtime cannot create a nested context; only this parent routes.
        let sender = self
            .desktop
            .sender
            .lock()
            .await
            .clone()
            .context("context_node_offline")?;
        let secret = self.desktop_secret.lock().await;
        runtime
            .connect(sender, secret.as_deref().context("context_node_offline")?)
            .await
            .context(Stage("context_connect"))?;
        contexts
            .runtimes
            .insert(authority.context_id.clone(), runtime.clone());
        Ok(runtime)
    }
}
