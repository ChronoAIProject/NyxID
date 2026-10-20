//! Shell-free orchestration for the minimal node image. The normal register
//! and start commands retain ownership of credential custody and agent startup.
use std::future::Future;
use std::path::Path;

use anyhow::{Result, bail};
use clap::Parser;
use zeroize::Zeroizing;

use crate::cli::NodeCommands;

#[derive(Parser)]
struct StartParser {
    #[command(subcommand)]
    command: NodeCommands,
}

pub(super) async fn run(args: Vec<String>) -> Result<()> {
    let directory = std::env::var("NYXID_NODE_CONFIG_DIR")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "/app/config".into());
    let token = Zeroizing::new(std::env::var("NYXID_NODE_TOKEN").unwrap_or_default());
    let url = std::env::var("NYXID_NODE_URL")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "ws://localhost:3001/api/v1/nodes/ws".into());
    run_with(&directory, &token, &url, args, |command| {
        Box::pin(super::run(command))
    })
    .await
}

async fn run_with<F, Fut>(
    directory: &str,
    token: &str,
    url: &str,
    args: Vec<String>,
    mut dispatch: F,
) -> Result<()>
where
    F: FnMut(NodeCommands) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    // Parse forwarded arguments before registration consumes a one-time token.
    // Keep Start's actual clap schema authoritative, including future options.
    let start = StartParser::try_parse_from(
        ["node", "start", "--config", directory]
            .into_iter()
            .map(str::to_owned)
            .chain(args),
    )?
    .command;
    let config_file = Path::new(directory).join("config.toml");
    if !config_file.is_file()
        && !token.is_empty()
        && dispatch(NodeCommands::Register {
            token: token.into(),
            url: Some(url.into()),
            config: Some(directory.into()),
            keychain: false,
            profile: None,
        })
        .await
        .is_err()
    {
        // Registration errors may include upstream-controlled text. Do not
        // print it here, where it could echo the registration credential.
        bail!("Docker node registration failed; check the server URL and registration token");
    }
    if !config_file.is_file() {
        bail!(
            "No node config found. Mount an existing config at NYXID_NODE_CONFIG_DIR (default /app/config), or set NYXID_NODE_TOKEN and NYXID_NODE_URL to register."
        );
    }
    dispatch(start).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::future::ready;

    #[tokio::test]
    async fn docker_entrypoint_existing_config_starts_without_registering() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("config.toml"), "existing").unwrap();
        let calls = RefCell::new(Vec::new());
        run_with(
            directory.path().to_str().unwrap(),
            "unused-registration-input",
            "ws://unused.invalid",
            vec!["--log-level".into(), "debug".into()],
            |command| {
                calls.borrow_mut().push(command);
                ready(Ok(()))
            },
        )
        .await
        .unwrap();
        let calls = calls.borrow();
        assert_eq!(calls.len(), 1);
        assert!(
            matches!(&calls[0], NodeCommands::Start { config: Some(config), log_level: Some(level), .. }
            if config == directory.path().to_str().unwrap() && level == "debug")
        );
    }

    #[tokio::test]
    async fn docker_entrypoint_missing_config_registers_with_file_custody_then_starts() {
        let directory = tempfile::tempdir().unwrap();
        let calls = RefCell::new(Vec::new());
        run_with(
            directory.path().to_str().unwrap(),
            "registration-fixture-value",
            "ws://fixture.invalid/api/v1/nodes/ws",
            vec![],
            |command| {
                if let NodeCommands::Register {
                    token,
                    url,
                    config,
                    keychain,
                    profile,
                } = &command
                {
                    assert_eq!(token, "registration-fixture-value");
                    assert_eq!(url.as_deref(), Some("ws://fixture.invalid/api/v1/nodes/ws"));
                    assert_eq!(config.as_deref(), directory.path().to_str());
                    assert!(!keychain);
                    assert!(profile.is_none());
                    std::fs::write(directory.path().join("config.toml"), "registered").unwrap();
                }
                calls.borrow_mut().push(command);
                ready(Ok(()))
            },
        )
        .await
        .unwrap();
        let calls = calls.borrow();
        assert_eq!(calls.len(), 2);
        assert!(matches!(calls[0], NodeCommands::Register { .. }));
        assert!(matches!(calls[1], NodeCommands::Start { .. }));
    }

    #[tokio::test]
    async fn docker_entrypoint_no_config_or_token_refuses_without_dispatch_or_secret_output() {
        let directory = tempfile::tempdir().unwrap();
        let error = run_with(
            directory.path().to_str().unwrap(),
            "",
            "ws://unused.invalid",
            vec![],
            |_| -> std::future::Ready<Result<()>> {
                panic!("must not dispatch without config or token")
            },
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("No node config found"));
        assert!(
            !error
                .to_string()
                .contains(directory.path().to_str().unwrap())
        );
    }

    #[tokio::test]
    async fn docker_entrypoint_registration_failure_cannot_echo_the_token() {
        let directory = tempfile::tempdir().unwrap();
        let secret = "registration-fixture-private";
        let error = run_with(
            directory.path().to_str().unwrap(),
            secret,
            "ws://fixture.invalid",
            vec![],
            |_| ready(Err(anyhow::anyhow!("server echoed {secret}"))),
        )
        .await
        .unwrap_err();
        assert!(!format!("{error:#}").contains(secret));
        assert!(error.to_string().contains("registration failed"));
        assert!(!directory.path().join("config.toml").exists());
    }

    #[tokio::test]
    async fn docker_entrypoint_invalid_forwarded_args_do_not_consume_registration_token() {
        let directory = tempfile::tempdir().unwrap();
        let result = run_with(
            directory.path().to_str().unwrap(),
            "fixture-token",
            "ws://fixture.invalid",
            vec!["--unknown-start-option".into()],
            |_| -> std::future::Ready<Result<()>> { panic!("must parse before registration") },
        )
        .await;
        assert!(result.is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "requires a real unlocked Secret Service in a disposable D-Bus session"]
    fn linux_keychain_roundtrip_inside_cli_tokio_runtime() {
        #[tokio::main]
        async fn exercise() {
            let id = format!("nyxid-w7-native-{}", uuid::Uuid::new_v4());
            let backend = crate::node::keychain::KeychainBackend::new(&id);
            backend.preflight().unwrap();
            let value = Zeroizing::new(uuid::Uuid::new_v4().to_string());
            backend.set("roundtrip", &value).unwrap();
            assert_eq!(backend.get("roundtrip").unwrap(), *value);
            backend.delete("roundtrip").unwrap();
            assert!(backend.get_optional("roundtrip").unwrap().is_none());
            println!(
                "Real Secret Service preflight/set/get/delete passed inside the CLI Tokio runtime"
            );
        }
        exercise();
    }
}
