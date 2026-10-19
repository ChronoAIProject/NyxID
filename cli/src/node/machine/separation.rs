//! Observe the mechanisms this runtime implements. An OS name, configured user
//! or kernel version is not evidence that a policy can actually be enforced.
use super::process::Identity;
use nyxid_machine::{config::Config, context::Evidence};

#[cfg(any(target_os = "linux", test))]
fn separate_users(supervisor: u32, command: (u32, u32), browsers: &[(u32, u32)]) -> bool {
    supervisor == 0
        && browsers.len() == 2
        && command.0 != 0
        && command.1 != 0
        && browsers.iter().enumerate().all(|(index, browser)| {
            browser.0 != 0
                && browser.1 != 0
                && browser.0 != command.0
                && browser.1 != command.1
                && browsers[..index]
                    .iter()
                    .all(|previous| previous.0 != browser.0 && previous.1 != browser.1)
        })
}

#[cfg(any(target_os = "linux", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Probe {
    CommandSeccomp,
    Landlock,
    SecureNamespaces,
    DeveloperNamespaces,
}

#[cfg(any(target_os = "linux", test))]
async fn run_probes<F, Fut>(mut evidence: Evidence, mut run: F) -> Evidence
where
    F: FnMut(Probe) -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    if evidence
        .landlock_abi
        .is_none_or(|abi| abi < nyxid_machine::context::MIN_LANDLOCK_ABI)
        || evidence.separate_users != Some(true)
        || evidence.managed_browser_configured != Some(true)
    {
        return evidence;
    }
    evidence.command_seccomp = Some(run(Probe::CommandSeccomp).await);
    if evidence.command_seccomp != Some(true) {
        return evidence;
    }
    evidence.landlock_enforced = Some(run(Probe::Landlock).await);
    if evidence.landlock_enforced != Some(true) {
        return evidence;
    }
    let secure = run(Probe::SecureNamespaces).await;
    let developer = secure && run(Probe::DeveloperNamespaces).await;
    evidence.browser_user_namespaces = Some(secure && developer);
    evidence
}

pub(super) async fn observe(
    config: &Config,
    command: &Identity,
    secure: &Identity,
    developer: Option<&Identity>,
) -> Evidence {
    let evidence = Evidence {
        version: nyxid_machine::context::EVIDENCE_VERSION,
        os: std::env::consts::OS.into(),
        ..Default::default()
    };
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (config, command, secure, developer);
        // No supported macOS context sandbox exists in this runtime. Separate
        // browser applications or TCC grants cannot turn this into Linux mode.
        evidence
    }
    #[cfg(target_os = "linux")]
    {
        use super::process;
        let mut evidence = evidence;
        evidence.landlock_abi = process::context_sandbox::abi()
            .ok()
            .and_then(|abi| u32::try_from(abi).ok());
        let browsers = developer.map(|dev| [(secure.uid, secure.gid), (dev.uid, dev.gid)]);
        evidence.separate_users = Some(separate_users(
            unsafe { libc::geteuid() },
            (command.uid, command.gid),
            browsers.as_ref().map_or(&[], |roles| roles.as_slice()),
        ));
        evidence.managed_browser_configured = Some(config.managed_browser.is_some());
        run_probes(evidence, |probe| async move {
            match probe {
                Probe::CommandSeccomp => process::probe_command_seccomp(command).await.is_ok(),
                Probe::Landlock => tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    process::context_sandbox::probe(command),
                )
                .await
                .is_ok_and(|result| result.is_ok()),
                Probe::SecureNamespaces => process::probe_browser_namespaces(secure).await.is_ok(),
                Probe::DeveloperNamespaces => match developer {
                    Some(developer) => process::probe_browser_namespaces(developer).await.is_ok(),
                    None => false,
                },
            }
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prerequisites() -> Evidence {
        Evidence {
            version: nyxid_machine::context::EVIDENCE_VERSION,
            os: "linux".into(),
            landlock_abi: Some(6),
            separate_users: Some(true),
            managed_browser_configured: Some(true),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn failed_probe_never_advertises_support_or_runs_later_probes() {
        let probes = [
            Probe::CommandSeccomp,
            Probe::Landlock,
            Probe::SecureNamespaces,
            Probe::DeveloperNamespaces,
        ];
        for (index, failed) in probes.iter().enumerate() {
            let mut calls = Vec::new();
            let evidence = run_probes(prerequisites(), |probe| {
                calls.push(probe);
                std::future::ready(probe != *failed)
            })
            .await;
            assert_eq!(calls, probes[..=index]);
            assert!(!evidence.support().available);
            assert_eq!(
                evidence.support().reason.as_deref(),
                Some(match failed {
                    Probe::CommandSeccomp => "separated_seccomp_probe_failed",
                    Probe::Landlock => "separated_landlock_probe_failed",
                    _ => "separated_browser_namespace_probe_failed",
                })
            );
        }
    }

    #[tokio::test]
    async fn configured_prerequisites_alone_cannot_advertise_separation() {
        assert!(!prerequisites().support().available);
        let mut calls = Vec::new();
        let evidence = run_probes(prerequisites(), |probe| {
            calls.push(probe);
            std::future::ready(true)
        })
        .await;
        assert!(evidence.support().available);
        assert_eq!(
            calls,
            [
                Probe::CommandSeccomp,
                Probe::Landlock,
                Probe::SecureNamespaces,
                Probe::DeveloperNamespaces
            ]
        );
    }

    #[tokio::test]
    async fn missing_prerequisites_do_not_attempt_privileged_probes() {
        for evidence in [
            Evidence {
                landlock_abi: None,
                ..prerequisites()
            },
            Evidence {
                landlock_abi: Some(5),
                ..prerequisites()
            },
            Evidence {
                separate_users: Some(false),
                ..prerequisites()
            },
            Evidence {
                managed_browser_configured: Some(false),
                ..prerequisites()
            },
        ] {
            let evidence = run_probes(evidence, |_| async {
                panic!("must not probe an unconfigured host")
            })
            .await;
            assert!(!evidence.support().available);
            assert_eq!(evidence.command_seccomp, None);
            assert_eq!(evidence.landlock_enforced, None);
            assert_eq!(evidence.browser_user_namespaces, None);
        }
    }

    #[test]
    fn separate_users_requires_supervisor_and_three_distinct_non_root_roles() {
        assert!(separate_users(
            0,
            (1000, 1000),
            &[(1001, 1001), (1002, 1002)]
        ));
        for (supervisor, command, browsers) in [
            (1000, (1000, 1000), vec![(1001, 1001), (1002, 1002)]),
            (0, (0, 1000), vec![(1001, 1001), (1002, 1002)]),
            (0, (1000, 0), vec![(1001, 1001), (1002, 1002)]),
            (0, (1000, 1000), vec![(1001, 1001)]),
            (0, (1000, 1000), vec![(0, 1001), (1002, 1002)]),
            (0, (1000, 1000), vec![(1001, 0), (1002, 1002)]),
            (0, (1000, 1000), vec![(1000, 1001), (1002, 1002)]),
            (0, (1000, 1000), vec![(1001, 1000), (1002, 1002)]),
            (0, (1000, 1000), vec![(1001, 1001), (1001, 1002)]),
            (0, (1000, 1000), vec![(1001, 1001), (1002, 1001)]),
        ] {
            assert!(!separate_users(supervisor, command, &browsers));
        }
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn native_macos_never_advertises_linux_context_separation() {
        let identity = Identity::resolve(None).unwrap();
        let evidence = observe(&Config::default(), &identity, &identity, None).await;
        assert_eq!(evidence.os, "macos");
        assert_eq!(
            evidence.unavailable_reason(),
            Some("separated_requires_linux")
        );
        assert!(!evidence.support().available);
        assert_eq!(evidence.landlock_abi, None);
        assert_eq!(evidence.command_seccomp, None);
        assert_eq!(evidence.browser_user_namespaces, None);
    }
}
