use crate::{
    api::{ApiClient, ApiError},
    cli::{ApproverCommands, ApproverEdit, OutputFormat},
    org_resolver::resolve_org_id,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use uuid::Uuid;

#[derive(Serialize, Deserialize)]
struct Candidate {
    user_id: Uuid,
    display_name: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Policy {
    config_id: Uuid,
    revision: i64,
    #[serde(deserialize_with = "serde::Deserialize::deserialize")]
    approver_user_ids: Option<Vec<Uuid>>,
    authoring_available: bool,
    eligible_candidates: Vec<Candidate>,
}

impl Policy {
    fn edited_ids(&self, edit: ApproverEdit) -> Result<Option<Vec<Uuid>>> {
        if edit.inherit {
            return Ok(None);
        }
        if !edit.set.is_empty() {
            return Ok(Some(edit.set));
        }
        ensure!(
            !edit.add.is_empty() || !edit.remove.is_empty(),
            "Choose --set, --inherit, --add or --remove"
        );
        let mut ids: BTreeSet<_> = match &self.approver_user_ids {
            Some(ids) => ids.iter().copied().collect(),
            None => self.eligible_candidates.iter().map(|c| c.user_id).collect(),
        };
        ensure!(
            edit.add.iter().all(|id| !edit.remove.contains(id)),
            "A person cannot be both added and removed"
        );
        let mut changed = false;
        for id in edit.add {
            changed |= ids.insert(id);
        }
        for id in edit.remove {
            changed |= ids.remove(&id);
        }
        if !changed {
            return Ok(self.approver_user_ids.clone());
        }
        Ok(Some(ids.into_iter().collect()))
    }

    fn render(&self, output: OutputFormat) -> Result<String> {
        if matches!(output, OutputFormat::Json) {
            return Ok(serde_json::to_string_pretty(self)?);
        }
        let selection = match &self.approver_user_ids {
            None => "Inherited (all eligible Admins)".to_owned(),
            Some(ids) if ids.is_empty() => "No named approvers remain".to_owned(),
            Some(ids) => ids
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", "),
        };
        let mut lines = vec![
            format!("Config {} (revision {})", self.config_id, self.revision),
            format!("Approvers: {selection}"),
            format!("Authoring available: {}", self.authoring_available),
            format!("Eligible Admins: {}", self.eligible_candidates.len()),
        ];
        for person in &self.eligible_candidates {
            lines.push(format!(
                "{}  {}",
                person.user_id,
                person.display_name.as_deref().unwrap_or("")
            ));
        }
        Ok(lines.join("\n"))
    }
}

pub(super) async fn run(command: ApproverCommands) -> Result<()> {
    let (target, edit) = match command {
        ApproverCommands::Show(target) => (target, None),
        ApproverCommands::Set { target, edit } => (target, Some(edit)),
    };
    let mut api = ApiClient::from_auth_checked(&target.auth).await?;
    let org = resolve_org_id(&mut api, &target.org).await?;
    let path = format!(
        "/approvals/service-configs/{}/approvers?org_id={}",
        urlencoding::encode(&target.service),
        urlencoding::encode(&org)
    );
    let mut policy: Policy = api.get(&path).await?;
    if let Some(edit) = edit {
        let ids = policy.edited_ids(edit)?;
        let body = serde_json::json!({"config_id":policy.config_id,"expected_revision":policy.revision,"approver_user_ids":ids});
        policy=match api.put(&path,&body).await {
            Err(error) if error.downcast_ref::<ApiError>().is_some_and(|e|e.status()==reqwest::StatusCode::CONFLICT) =>
                return Err(error).context("Approval config changed (409). Review it with nyxid approval approvers show before applying edits again"),
            result => result?,
        };
    }
    println!("{}", policy.render(target.auth.output)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        cli::{ApprovalCommands, Cli, Commands},
        test_support::mock_auth,
    };
    use clap::Parser;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, method, path, query_param},
    };

    fn parse(args: &[&str]) -> Result<ApproverCommands> {
        let cli = Cli::try_parse_from(
            ["nyxid", "approval", "approvers"]
                .into_iter()
                .chain(args.iter().copied()),
        )?;
        match cli.command {
            Commands::Approval {
                command: ApprovalCommands::Approvers { command },
            } => Ok(command),
            _ => unreachable!(),
        }
    }

    #[test]
    fn edit_modes_are_explicit_and_mutually_exclusive() {
        let org = Uuid::new_v4().to_string();
        let base = ["set", "--org", &org, "--service", "catalog"];
        for args in [
            vec![],
            vec!["--set", &org, "--inherit"],
            vec!["--set", &org, "--add", &org],
            vec!["--remove", &org, "--inherit"],
        ] {
            assert!(parse(&[base.as_slice(), &args].concat()).is_err());
        }
        let person = Uuid::new_v4();
        let current = Policy {
            config_id: person,
            revision: 4,
            approver_user_ids: None,
            authoring_available: true,
            eligible_candidates: vec![Candidate {
                user_id: person,
                display_name: Some("One".into()),
            }],
        };
        let edit = |args: &[&str]| match parse(&[base.as_slice(), args].concat()).unwrap() {
            ApproverCommands::Set { edit, .. } => edit,
            _ => unreachable!(),
        };
        assert!(
            current
                .render(OutputFormat::Table)
                .unwrap()
                .contains("Inherited")
        );
        assert_eq!(
            current
                .edited_ids(edit(&["--add", &person.to_string()]))
                .unwrap(),
            None
        );
        assert_eq!(
            current
                .edited_ids(edit(&["--remove", &person.to_string(), "--add", &org]))
                .unwrap(),
            Some(vec![org.parse().unwrap()])
        );
        assert_eq!(
            current.edited_ids(edit(&["--set", &org])).unwrap(),
            Some(vec![org.parse().unwrap()])
        );
        assert_eq!(current.edited_ids(edit(&["--inherit"])).unwrap(), None);
        assert!(
            current
                .edited_ids(edit(&["--add", &org, "--remove", &org]))
                .is_err()
        );
        let empty = Policy {
            approver_user_ids: Some(vec![]),
            ..current
        };
        assert!(
            empty
                .render(OutputFormat::Table)
                .unwrap()
                .contains("No named approvers")
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&empty.render(OutputFormat::Json).unwrap())
                .unwrap()["approver_user_ids"],
            json!([])
        );
    }

    #[tokio::test]
    async fn show_and_set_round_trip_use_org_lookup_revision_and_no_blind_retry() {
        let server = MockServer::start().await;
        let org = Uuid::new_v4();
        let config = Uuid::new_v4();
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        let route = "/api/v1/approvals/service-configs/catalog/approvers";
        let before = json!({"config_id":config,"revision":2,"approver_user_ids":null,"authoring_available":true,"eligible_candidates":[{"user_id":first,"display_name":null},{"user_id":second,"display_name":"Two"}]});
        let after = json!({"revision":3,"approver_user_ids":[second],
            "config_id":config,"authoring_available":true,"eligible_candidates":before["eligible_candidates"]});
        Mock::given(method("GET"))
            .and(path("/api/v1/orgs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"orgs":[{"id":org,"slug":"acme","display_name":"Acme Team"}]}),
            ))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/orgs/acme"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"id":org,"slug":"acme","display_name":"Acme Team"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(route))
            .and(query_param("org_id", org.to_string()))
            .respond_with(ResponseTemplate::new(200).set_body_json(&before))
            .up_to_n_times(2)
            .expect(2)
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path(route))
            .and(query_param("org_id", org.to_string()))
            .and(body_json(
                json!({"config_id":config,"expected_revision":2,"approver_user_ids":[second]}),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(&after))
            .expect(1)
            .mount(&server)
            .await;
        let invoke = |args: Vec<&str>| {
            let mut command = parse(&args).unwrap();
            let target = match &mut command {
                ApproverCommands::Show(t) | ApproverCommands::Set { target: t, .. } => t,
            };
            target.auth = mock_auth(server.uri());
            target.auth.output = OutputFormat::Json;
            run(command)
        };
        invoke(vec!["show", "--org", "Acme Team", "--service", "catalog"])
            .await
            .unwrap();
        invoke(vec![
            "set",
            "--org",
            "acme",
            "--service",
            "catalog",
            "--remove",
            &first.to_string(),
        ])
        .await
        .unwrap();

        Mock::given(method("GET"))
            .and(path(route))
            .and(query_param("org_id", org.to_string()))
            .respond_with(ResponseTemplate::new(200).set_body_json(&after))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path(route))
            .and(query_param("org_id", org.to_string()))
            .and(body_json(
                json!({"config_id":config,"expected_revision":3,"approver_user_ids":null}),
            ))
            .respond_with(ResponseTemplate::new(409))
            .expect(1)
            .mount(&server)
            .await;
        let err = invoke(vec![
            "set",
            "--org",
            &org.to_string(),
            "--service",
            "catalog",
            "--inherit",
        ])
        .await
        .unwrap_err();
        assert!(err.to_string().contains("409"));
    }
}
