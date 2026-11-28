use super::*;

#[tokio::test]
async fn skill_read_builtin_guid_beats_attached_alias_on_all_paths() {
    use crate::services::assistant_team_service as team;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    // Four distinct archives ensure that a wrong but authorized root/dependency
    // cannot pass by sharing the expected fixture bytes or hashes.
    const ROOT: &str = "## Overview\nExact built-in result 中文.\n\n## Approach\n1. **Read.** Follow the exact root.\n";
    const DEPENDENCY: &str = "## Overview\nExact dependency result 中文.\n\n## Approach\n1. **Check.** Follow the exact dependency.\n";
    for specialist in [false, true] {
        let f = if specialist {
            fixture("builtin_guid_alias_specialist").await
        } else {
            orchestrator_fixture("builtin_guid_alias_owner").await
        };
        let db = &f.state.db;
        flag(db, &f.owner, true).await;
        flags::set_platform_override(
            db,
            flags::SKILL_RUNS_FLAG_KEY,
            &flags::FlagTarget::User(f.owner.clone()),
            true,
            &f.owner,
        )
        .await
        .unwrap();
        let root_bytes = Registry::with_markdown(ROOT).bytes;
        let dep_bytes = Registry::with_markdown(DEPENDENCY).bytes;
        let alias_bytes = Registry::with_markdown("Wrong attached alias root").bytes;
        let alias_dep_bytes = Registry::with_markdown("Wrong dependency alias").bytes;
        let mut root = Registry::new().reference();
        root.sha256 = hex::encode(Sha256::digest(&root_bytes));
        root.dependencies[0].sha256 = hex::encode(Sha256::digest(&dep_bytes));
        let mut alias = root.clone();
        alias.skill_id = "00000000-0000-4000-8000-000000000003".into();
        alias.name = ID.into();
        alias.sha256 = hex::encode(Sha256::digest(&alias_bytes));
        alias.dependencies.clear();
        let mut dep_alias = root.dependencies[0].clone();
        dep_alias.skill_id = "00000000-0000-4000-8000-000000000004".into();
        dep_alias.name = DEP.into();
        dep_alias.sha256 = hex::encode(Sha256::digest(&alias_dep_bytes));
        root.dependencies.insert(0, dep_alias.clone());
        let mut configured = system(&Registry::new());
        configured.skills[0].reference = root.clone();
        db.collection(COLLECTION_NAME)
            .insert_one(configured)
            .await
            .unwrap();
        db.collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&f.chat.agent_id},
                doc! {"$set":{"skills":[bson::to_bson(&alias).unwrap()]}},
            )
            .await
            .unwrap();
        let server = MockServer::start().await;
        for (id, bytes) in [
            (ID, root_bytes),
            (DEP, dep_bytes),
            (alias.skill_id.as_str(), alias_bytes),
            (dep_alias.skill_id.as_str(), alias_dep_bytes),
        ] {
            Mock::given(method("GET"))
                .and(path(format!("/api/v1/skills/{id}/versions/1.0/download")))
                .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes))
                .mount(&server)
                .await;
        }
        let mut catalog = crate::test_utils::test_auto_connected_catalog_service();
        catalog.slug = "ornn-api".into();
        catalog.base_url = server.uri();
        catalog.identity_propagation_mode = "jwt".into();
        db.collection::<crate::models::downstream_service::DownstreamService>(
            crate::models::downstream_service::COLLECTION_NAME,
        )
        .insert_one(catalog)
        .await
        .unwrap();
        let args = json!({"skill":ID,"path":"/"});
        let (manifest, error) = crate::handlers::assistant_team::execute_tool(
            &f.state,
            &f.chat,
            "nyxid__skill_read",
            &args,
        )
        .await;
        assert!(!error, "{manifest}");
        assert_eq!(manifest["skill_id"], ID);
        assert_eq!(manifest["archive_sha256"], root.sha256);
        let source = &manifest["source_fingerprint"];
        let args = json!({"agent":f.chat.agent_id,"skill_id":ID});
        let (pin, error) = crate::handlers::assistant_team::execute_tool(
            &f.state,
            &f.chat,
            "nyxid__get_agent_skills",
            &args,
        )
        .await;
        assert!(!error, "{pin}");
        assert_eq!(pin["pin"], json!(root));
        assert_eq!(pin["provenance"], "built_in");
        assert_eq!(pin["effective"], true);
        for (dep, expected, hash) in [
            (None, ROOT, root.sha256.as_str()),
            (Some(DEP), DEPENDENCY, root.dependencies[1].sha256.as_str()),
        ] {
            let mut args = json!({"skill":ID,"source_fingerprint":source});
            if let Some(dep) = dep {
                args["dependency"] = json!(dep);
            }
            let (page, error) = crate::handlers::assistant_team::execute_tool(
                &f.state,
                &f.chat,
                "nyxid__skill_read",
                &args,
            )
            .await;
            assert!(!error, "{page}");
            assert_eq!(page["content"], expected);
            assert_eq!(
                page["sha256"],
                hex::encode(Sha256::digest(expected.as_bytes()))
            );
            assert_eq!(page["archive_sha256"], hash);
            assert_eq!(page["source_fingerprint"], *source);
            let mut args = json!({"skill":ID,"section":"outline"});
            if let Some(dep) = dep {
                args["dependency"] = json!(dep);
            }
            let (outline, error) = crate::handlers::assistant_team::execute_tool(
                &f.state,
                &f.chat,
                "nyxid__skill_read",
                &args,
            )
            .await;
            assert!(!error, "{outline}");
            let expected_outline = crate::services::skill_outline::render(
                &crate::services::skill_outline::parse(expected, &root.dependencies).unwrap(),
            );
            assert_eq!(outline["content"], expected_outline);
        }
        let agent = team::agent(db, &f.owner, &f.chat.agent_id).await.unwrap();
        let context = for_person(db, &f.owner, false).await.unwrap().unwrap();
        assert_eq!(resolve(&agent, Some(&context), ID), Some(&root));
        let bound =
            crate::services::agent_skill_service::OrnnSkillRead::new(&agent.id, root.clone());
        for id in [ID, DEP] {
            bound
                .revalidate(
                    db,
                    &f.owner,
                    Some(&f.chat),
                    None,
                    "GET",
                    &format!("/api/v1/skills/{id}/versions/1.0/download"),
                )
                .await
                .unwrap();
        }
        assert!(
            bound
                .revalidate(
                    db,
                    &f.owner,
                    Some(&f.chat),
                    None,
                    "GET",
                    &format!("/api/v1/skills/{}/versions/1.0/download", alias.skill_id)
                )
                .await
                .is_err()
        );
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 5);
        assert!(requests.iter().all(|request| {
            [ID, DEP].iter().any(|id| {
                request.url.path() == format!("/api/v1/skills/{id}/versions/1.0/download")
            })
        }));
        // Disabling the built-in cannot fall back to the differently identified alias
        // under the old source; selected download authority is independently live.
        flag(db, &f.owner, false).await;
        assert!(
            bound
                .revalidate(
                    db,
                    &f.owner,
                    Some(&f.chat),
                    None,
                    "GET",
                    &format!("/api/v1/skills/{ID}/versions/1.0/download")
                )
                .await
                .is_err()
        );
        db.drop().await.unwrap();
    }
}
