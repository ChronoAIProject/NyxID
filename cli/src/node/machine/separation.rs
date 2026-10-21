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
    BrowserPolicy,
    SecureBrowser,
    DeveloperBrowser,
}

#[cfg(any(target_os = "linux", test))]
#[derive(Debug, thiserror::Error)]
#[error("separated_browser_sandbox_unobservable_in_container")]
pub(super) struct BrowserSandboxUnobservableInContainer;

#[cfg(any(target_os = "linux", test))]
async fn run_probes<F, Fut>(mut evidence: Evidence, mut run: F) -> Evidence
where
    F: FnMut(Probe) -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<()>>,
{
    if evidence
        .landlock_abi
        .is_none_or(|abi| abi < nyxid_machine::context::MIN_LANDLOCK_ABI)
        || evidence.separate_users != Some(true)
        || evidence.managed_browser_configured != Some(true)
    {
        return evidence;
    }
    for probe in [
        Probe::CommandSeccomp,
        Probe::Landlock,
        Probe::BrowserPolicy,
        Probe::SecureBrowser,
        Probe::DeveloperBrowser,
    ] {
        let started = std::time::Instant::now();
        let result = run(probe).await;
        eprintln!(
            "separation_probe mechanism={probe:?} elapsed_ms={} outcome={}",
            started.elapsed().as_millis(),
            if result.is_ok() {
                "confirmed"
            } else {
                "failed"
            }
        );
        let succeeded = result.is_ok();
        if let Err(error) = result {
            evidence.probe_timed_out = error.is::<super::process::ProbeTimeout>();
            if error.is::<BrowserSandboxUnobservableInContainer>() {
                evidence.browser_sandbox_unobservable_in_container = true;
                evidence.managed_browser_sandbox = Some(false);
                eprintln!(
                    "separation_probe_unavailable reason=separated_browser_sandbox_unobservable_in_container"
                );
            }
        }
        match probe {
            Probe::CommandSeccomp => evidence.command_seccomp = Some(succeeded),
            Probe::Landlock => evidence.landlock_enforced = Some(succeeded),
            Probe::BrowserPolicy => evidence.managed_browser_policy = Some(succeeded),
            Probe::SecureBrowser | Probe::DeveloperBrowser => {
                evidence.managed_browser_sandbox = Some(succeeded);
            }
        }
        if !succeeded {
            break;
        }
    }
    evidence
}

/// Read Chromium's built-in renderer sandbox diagnostics after launching the
/// configured executable. Unknown/partial pages and duplicate rows fail closed.
#[cfg(test)]
fn chromium_sandbox_active(output: &[u8]) -> bool {
    static ROW: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r"(?s)<tr>\s*<td(?:\s[^>]*)?>([^<]*)</td>\s*<td(?:\s[^>]*)?>([^<]*)</td>\s*</tr>",
        )
        .expect("constant sandbox row pattern")
    });
    let Ok(html) = std::str::from_utf8(output) else {
        return false;
    };
    let rows: Vec<_> = ROW
        .captures_iter(html)
        .map(|row| (row[1].trim().to_owned(), row[2].trim().to_owned()))
        .collect();
    [
        ("Layer 1 Sandbox", "Namespace"),
        ("PID namespaces", "Yes"),
        ("Network namespaces", "Yes"),
        ("Seccomp-BPF sandbox", "Yes"),
    ]
    .into_iter()
    .all(|(name, value)| {
        let mut matches = rows.iter().filter(|row| row.0 == name);
        matches.next().is_some_and(|row| row.1 == value) && matches.next().is_none()
    })
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
        evidence.apparmor_userns_restricted =
            std::fs::read_to_string("/proc/sys/kernel/apparmor_restrict_unprivileged_userns")
                .ok()
                .and_then(|value| match value.trim() {
                    "0" => Some(false),
                    "1" => Some(true),
                    _ => None,
                });
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
                Probe::CommandSeccomp => process::probe_command_seccomp(command).await,
                Probe::Landlock => process::context_sandbox::probe(command).await,
                Probe::BrowserPolicy => match config.managed_browser.as_ref() {
                    Some(browser) => super::browser_probe::observe(browser, secure, true).await,
                    None => anyhow::bail!("separated_browser_identity_missing"),
                },
                Probe::SecureBrowser | Probe::DeveloperBrowser => {
                    let identity = if probe == Probe::SecureBrowser {
                        Some(secure)
                    } else {
                        developer
                    };
                    match (config.managed_browser.as_ref(), identity) {
                        (Some(browser), Some(identity)) => {
                            super::browser_probe::observe(browser, identity, false).await
                        }
                        _ => anyhow::bail!("separated_browser_identity_missing"),
                    }
                }
            }
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sandbox_page() -> String {
        [
            ("Layer 1 Sandbox", "Namespace"),
            ("PID namespaces", "Yes"),
            ("Network namespaces", "Yes"),
            ("Seccomp-BPF sandbox", "Yes"),
        ]
        .into_iter()
        .map(|(name, value)| {
            format!("<tr><td class=\"good\">{name}</td><td class=\"good\">{value}</td></tr>")
        })
        .collect()
    }

    #[test]
    fn chromium_diagnostics_require_every_active_sandbox_layer() {
        let good = sandbox_page();
        assert!(chromium_sandbox_active(good.as_bytes()));
        for (name, value) in [
            ("Layer 1 Sandbox", "Namespace"),
            ("PID namespaces", "Yes"),
            ("Network namespaces", "Yes"),
            ("Seccomp-BPF sandbox", "Yes"),
        ] {
            let row =
                format!("<tr><td class=\"good\">{name}</td><td class=\"good\">{value}</td></tr>");
            assert!(
                !chromium_sandbox_active(good.replace(&row, "").as_bytes()),
                "{name}"
            );
            assert!(
                !chromium_sandbox_active(good.replace(&row, &row.replace(value, "No")).as_bytes()),
                "{name}"
            );
            assert!(
                !chromium_sandbox_active(format!("{good}{row}").as_bytes()),
                "{name}"
            );
        }
        assert!(!chromium_sandbox_active(
            good.replace("Namespace", "SUID").as_bytes()
        ));
        assert!(!chromium_sandbox_active(
            b"<title>chrome://sandbox failed</title>"
        ));
        assert!(!chromium_sandbox_active(b"Layer 1 Sandbox Namespace PID namespaces Yes Network namespaces Yes Seccomp-BPF sandbox Yes"));
        assert!(!chromium_sandbox_active(b"\xff"));
    }

    fn prerequisites() -> Evidence {
        Evidence {
            version: nyxid_machine::context::EVIDENCE_VERSION,
            os: "linux".into(),
            landlock_abi: Some(6),
            separate_users: Some(true),
            credential_custody: Some(true),
            managed_browser_configured: Some(true),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn failed_probe_never_advertises_support_or_runs_later_probes() {
        let probes = [
            Probe::CommandSeccomp,
            Probe::Landlock,
            Probe::BrowserPolicy,
            Probe::SecureBrowser,
            Probe::DeveloperBrowser,
        ];
        for (index, failed) in probes.iter().enumerate() {
            let mut calls = Vec::new();
            let evidence = run_probes(prerequisites(), |probe| {
                calls.push(probe);
                std::future::ready(if probe == *failed {
                    Err(anyhow::anyhow!("test probe denied"))
                } else {
                    Ok(())
                })
            })
            .await;
            assert_eq!(calls, probes[..=index]);
            assert!(!evidence.support().available);
            assert_eq!(
                evidence.support().reason.as_deref(),
                Some(match failed {
                    Probe::CommandSeccomp => "separated_seccomp_probe_failed",
                    Probe::Landlock => "separated_landlock_probe_failed",
                    Probe::BrowserPolicy => "separated_browser_policy_not_effective",
                    _ => "separated_browser_sandbox_probe_failed",
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
            std::future::ready(Ok(()))
        })
        .await;
        assert!(evidence.support().available);
        assert_eq!(
            calls,
            [
                Probe::CommandSeccomp,
                Probe::Landlock,
                Probe::BrowserPolicy,
                Probe::SecureBrowser,
                Probe::DeveloperBrowser
            ]
        );
    }

    #[tokio::test]
    async fn each_probe_timeout_is_advertised_distinctly_and_stops_later_probes() {
        for timed_out in [
            Probe::CommandSeccomp,
            Probe::Landlock,
            Probe::BrowserPolicy,
            Probe::SecureBrowser,
            Probe::DeveloperBrowser,
        ] {
            let mut calls = Vec::new();
            let evidence = run_probes(prerequisites(), |probe| {
                calls.push(probe);
                std::future::ready(if probe == timed_out {
                    Err(super::super::process::ProbeTimeout.into())
                } else {
                    Ok(())
                })
            })
            .await;
            assert_eq!(calls.last(), Some(&timed_out));
            assert!(evidence.probe_timed_out);
            assert!(!evidence.support().available);
            assert_eq!(
                evidence.support().reason.as_deref(),
                Some("separated_probe_timeout")
            );
        }
    }

    #[tokio::test]
    async fn unobservable_container_browser_explicitly_denies_sandbox_and_stops_probes() {
        for failed in [
            Probe::BrowserPolicy,
            Probe::SecureBrowser,
            Probe::DeveloperBrowser,
        ] {
            let mut calls = Vec::new();
            let evidence = run_probes(prerequisites(), |probe| {
                calls.push(probe);
                std::future::ready(if probe == failed {
                    Err(anyhow::Error::new(super::super::process::ProbeTimeout)
                        .context(BrowserSandboxUnobservableInContainer))
                } else {
                    Ok(())
                })
            })
            .await;
            assert_eq!(calls.last(), Some(&failed));
            assert_eq!(evidence.managed_browser_sandbox, Some(false));
            assert!(evidence.browser_sandbox_unobservable_in_container);
            assert!(evidence.probe_timed_out);
            assert_eq!(
                evidence.support().reason.as_deref(),
                Some("separated_browser_sandbox_unobservable_in_container")
            );
            assert!(!evidence.support().available);
        }
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
            assert_eq!(evidence.managed_browser_sandbox, None);
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
        assert_eq!(evidence.managed_browser_sandbox, None);
    }
}
