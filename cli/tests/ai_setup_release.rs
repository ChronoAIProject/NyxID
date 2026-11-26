#![cfg(unix)]

use std::{fs, os::unix::fs::PermissionsExt, path::Path};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    process::Command,
};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path, path_regex, query_param},
};

const HOSTED: &str = "https://nyx-api.chrono-ai.fun";
const RELEASE: &str = concat!("v", env!("CARGO_PKG_VERSION"));

// Terminate the CLI's HTTPS CONNECTs locally so the real fetches exercise the
// release URLs. Every tunnel ends at the same loopback wiremock server.
async fn release_proxy(server: &MockServer, cert_path: &Path) -> String {
    let hosts = [
        "api.github.com",
        "raw.githubusercontent.com",
        "nyx-api.chrono-ai.fun",
    ];
    let cert = rcgen::generate_simple_self_signed(hosts.map(String::from)).unwrap();
    fs::write(cert_path, cert.cert.pem()).unwrap();
    let tls = rustls::ServerConfig::builder_with_provider(
        rustls::crypto::aws_lc_rs::default_provider().into(),
    )
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![cert.cert.der().clone()],
        rustls::pki_types::PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der()).into(),
    )
    .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(std::sync::Arc::new(tls));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let upstream = *server.address();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                let mut stream = BufReader::new(stream);
                let mut line = String::new();
                stream.read_line(&mut line).await.unwrap();
                assert!(
                    hosts
                        .iter()
                        .any(|host| line == format!("CONNECT {host}:443 HTTP/1.1\r\n")),
                    "{line}"
                );
                loop {
                    line.clear();
                    if stream.read_line(&mut line).await.unwrap() == 0 {
                        return;
                    }
                    if line == "\r\n" {
                        break;
                    }
                }
                stream.write_all(b"HTTP/1.1 200 OK\r\n\r\n").await.unwrap();
                let mut tls = acceptor.accept(stream).await.unwrap();
                let mut http = TcpStream::connect(upstream).await.unwrap();
                let _ = tokio::io::copy_bidirectional(&mut tls, &mut http).await;
            });
        }
    });
    format!("http://{address}")
}

struct Install {
    home: tempfile::TempDir,
    server: MockServer,
    proxy: String,
}

impl Install {
    async fn new() -> Self {
        let home = tempfile::tempdir().unwrap();
        let server = MockServer::start().await;
        let proxy = release_proxy(&server, &home.path().join("ca.pem")).await;
        Mock::given(method("GET"))
            .and(header("Host", "raw.githubusercontent.com"))
            .and(path_regex(format!(
                "^/ChronoAIProject/NyxID/{}/skills/",
                regex::escape(RELEASE)
            )))
            .respond_with(
                ResponseTemplate::new(200).set_body_string("Use https://nyx-api.chrono-ai.fun"),
            )
            .mount(&server)
            .await;
        Mock::given(path("/api/v1/public/config"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"frontend_url":"https://dashboard.example"})),
            )
            .mount(&server)
            .await;
        Mock::given(path("/llms.txt"))
            .respond_with(ResponseTemplate::new(200).set_body_string("instance playbook"))
            .mount(&server)
            .await;
        Self {
            home,
            server,
            proxy,
        }
    }

    fn command(&self, action: &str, tool: &str, base: &str) -> Command {
        let home = self.home.path();
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_nyxid"));
        cmd.args(["ai-setup", action, "--tool", tool, "--base-url", base])
            .current_dir(home)
            .env("HOME", home)
            .env("PATH", home)
            .env("CODEX_HOME", home.join(".agents"))
            .env("CARGO_HOME", home.join(".cargo"))
            .env("NYXID_CA_CERT", home.join("ca.pem"))
            .env("SSL_CERT_FILE", "")
            .env("SSL_CERT_DIR", "")
            .env("NO_PROXY", "127.0.0.1")
            .env("no_proxy", "127.0.0.1")
            .env("HTTPS_PROXY", &self.proxy)
            .env("https_proxy", &self.proxy)
            .env("DO_NOT_TRACK", "1")
            .env("NYXID_NO_UPDATE_CHECK", "1")
            .env("NYXID_SKIP_SKILL_SELF_HEAL", "1")
            .kill_on_drop(true);
        cmd
    }
}

#[tokio::test]
async fn codex_install_reads_tree_and_files_from_the_same_cli_release() {
    let install = Install::new().await;
    Mock::given(header("Host", "api.github.com"))
        .and(path(format!(
            "/repos/ChronoAIProject/NyxID/git/trees/{RELEASE}"
        )))
        .and(query_param("recursive", "1"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"tree":[
                {"type":"blob","path":"skills/nyxid/SKILL.md"},
                {"type":"blob","path":"skills/other/SKILL.md"}
            ]})),
        )
        .expect(1)
        .mount(&install.server)
        .await;
    let base = install.server.uri();
    let out = install
        .command("install", "codex", &base)
        .output()
        .await
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let root = install.home.path().join(".agents/skills");
    for skill in ["other", "nyxid"] {
        assert_eq!(
            fs::read_to_string(root.join(skill).join("SKILL.md")).unwrap(),
            format!("Use {base}")
        );
    }
    assert_eq!(
        fs::read_to_string(root.join("nyxid/references/playbook.md")).unwrap(),
        "instance playbook"
    );
}

#[tokio::test]
async fn claude_update_selects_and_confirms_the_official_release_before_updating() {
    let install = Install::new().await;
    let home = install.home.path();
    let registry = home.join(".claude/plugins");
    fs::create_dir_all(&registry).unwrap();
    fs::write(
        registry.join("installed_plugins.json"),
        r#"{"plugins":{"nyxid-cli@nyxid":[{"version":"1"}],"nyxid@nyxid":[{"version":"0.9.0"}]}}"#,
    )
    .unwrap();
    let claude = home.join("claude");
    fs::write(
        &claude,
        r#"#!/bin/sh
printf '%s\n' "$*" >> calls
case "$*" in
    '--version') exit 0 ;;
    'plugin marketplace list --json') /bin/cat marketplaces ;;
    *'plugin marketplace add '*) /bin/cp after-add marketplaces ;;
    'plugin update nyxid-cli@nyxid') exit 0 ;;
    *) exit 2 ;;
esac
"#,
    )
    .unwrap();
    fs::set_permissions(claude, fs::Permissions::from_mode(0o755)).unwrap();
    let official = |reference: &str| {
        serde_json::json!([{"name":"nyxid","source":"github","repo":"ChronoAIProject/NyxID","ref":reference}]).to_string()
    };
    let pinned = official(RELEASE);
    let old = official("main");
    let foreign = old.replace("ChronoAIProject/NyxID", "someone/nyxid");
    let add = format!("plugin marketplace add ChronoAIProject/NyxID#{RELEASE}");
    let list = "plugin marketplace list --json";
    let update = "plugin update nyxid-cli@nyxid";
    for (before, after, succeeded) in [
        (&old, &pinned, true),
        (&pinned, &pinned, true),
        (&foreign, &pinned, false),
        (&old, &old, false),
    ] {
        fs::write(home.join("marketplaces"), before).unwrap();
        fs::write(home.join("after-add"), after).unwrap();
        fs::write(home.join("calls"), "").unwrap();
        let out = install
            .command("update", "claude-code", HOSTED)
            .output()
            .await
            .unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.success(), succeeded, "{stderr}");
        let calls = fs::read_to_string(home.join("calls")).unwrap();
        let mut calls = calls.lines();
        assert_eq!(calls.next(), Some("--version"));
        assert_eq!(calls.next(), Some(list));
        if before == &old {
            let settings = calls
                .next()
                .unwrap()
                .strip_prefix("--settings ")
                .unwrap()
                .strip_suffix(&format!(" {add}"))
                .unwrap();
            let settings: serde_json::Value = serde_json::from_str(settings).unwrap();
            assert_eq!(
                settings["extraKnownMarketplaces"]["nyxid"]["source"],
                serde_json::json!({"source":"github","repo":"ChronoAIProject/NyxID","ref":RELEASE})
            );
            assert_eq!(calls.next(), Some(list));
        }
        if succeeded {
            assert_eq!(calls.next(), Some(update));
        } else if before == &foreign {
            assert!(stderr.contains("different source"), "{stderr}");
        } else {
            assert!(stderr.contains("could not select"), "{stderr}");
        }
        assert_eq!(calls.next(), None);
    }
}
