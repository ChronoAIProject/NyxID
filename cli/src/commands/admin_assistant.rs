use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::{
    api::ApiClient,
    cli::{AdminAssistantCommands, OutputFormat, SystemSkillsCommands},
};

const PATH: &str = "/admin/assistant/system-skills";

pub fn parse_skill(value: &str) -> Result<String, String> {
    let (skill, version) = value
        .rsplit_once('@')
        .ok_or("Use <skill-id-or-name>@<major.minor>")?;
    let parts: Vec<_> = version.split('.').collect();
    if skill.is_empty()
        || skill.len() > 80
        || !skill
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        || version.len() > 32
        || parts.len() != 2
        || parts.iter().any(|part| {
            part.is_empty()
                || (part.len() > 1 && part.starts_with('0'))
                || !part.bytes().all(|b| b.is_ascii_digit())
        })
    {
        return Err("Use <skill-id-or-name>@<exact major.minor>".into());
    }
    Ok(value.into())
}

pub async fn run(command: AdminAssistantCommands) -> Result<()> {
    match command {
        AdminAssistantCommands::SystemSkills { command } => {
            let (value, output) = match command {
                SystemSkillsCommands::Show { auth } => {
                    let mut api = ApiClient::from_auth_checked(&auth).await?;
                    (api.get::<Value>(PATH).await?, auth.output)
                }
                SystemSkillsCommands::Set {
                    skill,
                    clear: _,
                    auth,
                } => {
                    anyhow::ensure!(skill.len() <= 8, "At most 8 built-in skills are allowed");
                    let mut api = ApiClient::from_auth_checked(&auth).await?;
                    let current: Value = api.get(PATH).await?;
                    let revision = current["revision"]
                        .as_i64()
                        .context("Server omitted the system skills revision")?;
                    let entries: Vec<_> = skill
                        .iter()
                        .map(|value| {
                            let (skill, version) =
                                value.rsplit_once('@').expect("clap-validated skill");
                            json!({"skill":skill,"version":version})
                        })
                        .collect();
                    let saved = api
                        .put::<Value, _>(
                            PATH,
                            &json!({"expected_revision":revision,"skills":entries}),
                        )
                        .await?;
                    (saved, auth.output)
                }
            };
            if matches!(output, OutputFormat::Json) {
                println!("{}", serde_json::to_string_pretty(&value)?);
            } else {
                println!("Built-in skills · Revision {}", value["revision"]);
                for skill in value["skills"].as_array().into_iter().flatten() {
                    println!(
                        "{} @ {} · {} bytes\n  {}",
                        skill["name"].as_str().unwrap_or("-"),
                        skill["version"].as_str().unwrap_or("-"),
                        skill["size_bytes"],
                        skill["description"].as_str().unwrap_or("")
                    );
                }
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        cli::{AdminCommands, Cli, Commands},
        test_support::mock_auth,
    };
    use clap::Parser;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, method, path},
    };

    #[test]
    fn exact_pins_and_clear_are_required() {
        for value in ["nyxbot@1.0", "cma@12.34"] {
            assert!(parse_skill(value).is_ok());
        }
        for value in [
            "nyxbot",
            "@1.0",
            "nyxbot@latest",
            "nyxbot@1",
            "nyxbot@01.0",
            "nyxbot@1.0.0",
        ] {
            assert!(parse_skill(value).is_err());
        }
        assert!(
            Cli::try_parse_from(["nyxid", "admin", "assistant", "system-skills", "set"]).is_err()
        );
        assert!(
            Cli::try_parse_from([
                "nyxid",
                "admin",
                "assistant",
                "system-skills",
                "set",
                "--clear",
                "--skill",
                "cma@1.0"
            ])
            .is_err()
        );
    }

    #[tokio::test]
    async fn set_reads_revision_and_sends_only_selection() {
        let server = MockServer::start().await;
        let auth = mock_auth(server.uri());
        Mock::given(method("GET"))
            .and(path(format!("/api/v1{PATH}")))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"revision":7,"skills":[]})),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("PUT")).and(path(format!("/api/v1{PATH}")))
            .and(body_json(json!({"expected_revision":7,"skills":[{"skill":"nyxbot","version":"1.0"},{"skill":"cma","version":"2.0"}]})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"revision":8,"skills":[]}))).expect(1).mount(&server).await;
        run(AdminAssistantCommands::SystemSkills {
            command: SystemSkillsCommands::Set {
                skill: vec!["nyxbot@1.0".into(), "cma@2.0".into()],
                clear: false,
                auth,
            },
        })
        .await
        .unwrap();
    }

    #[test]
    fn commands_parse_json_and_repeatable_skills() {
        let cli = Cli::try_parse_from([
            "nyxid",
            "admin",
            "assistant",
            "system-skills",
            "set",
            "--skill",
            "nyxbot@1.0",
            "--skill",
            "cma@2.0",
            "--output",
            "json",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Admin {
                command: AdminCommands::Assistant { .. }
            }
        ));
    }
    #[tokio::test]
    async fn show_and_clear_use_the_admin_endpoint() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/api/v1{PATH}")))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"revision":2,"skills":[]})),
            )
            .expect(2)
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path(format!("/api/v1{PATH}")))
            .and(body_json(json!({"expected_revision":2,"skills":[]})))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"revision":3,"skills":[]})),
            )
            .expect(1)
            .mount(&server)
            .await;
        run(AdminAssistantCommands::SystemSkills {
            command: SystemSkillsCommands::Show {
                auth: mock_auth(server.uri()),
            },
        })
        .await
        .unwrap();
        run(AdminAssistantCommands::SystemSkills {
            command: SystemSkillsCommands::Set {
                skill: vec![],
                clear: true,
                auth: mock_auth(server.uri()),
            },
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn revision_conflict_is_returned_without_rebasing() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/api/v1{PATH}")))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"revision":1,"skills":[]})),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("PUT")).and(path(format!("/api/v1{PATH}")))
            .respond_with(ResponseTemplate::new(409).set_body_json(json!({"error":"conflict","error_code":1000,"message":"System skills changed; reload the current revision"})))
            .expect(1).mount(&server).await;
        assert!(
            run(AdminAssistantCommands::SystemSkills {
                command: SystemSkillsCommands::Set {
                    skill: vec![],
                    clear: true,
                    auth: mock_auth(server.uri())
                }
            })
            .await
            .is_err()
        );
    }
}
