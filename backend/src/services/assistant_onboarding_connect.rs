//! Live, owner-clicked setup choices. No links are minted on suggestion reads.
use crate::{
    errors::{AppError, AppResult},
    models::downstream_service::DownstreamService,
};
use futures::TryStreamExt;
use mongodb::{
    Database,
    bson::{Document, doc},
};

pub struct Choice {
    pub slug: String,
    pub name: String,
    pub scopes: Vec<String>,
}

pub struct Setup {
    pub connected: bool,
    pub choices: Vec<Choice>,
    pub prompt: String,
}

pub fn candidates(id: &str) -> &'static [&'static str] {
    match id {
        "connect_calendar" => &["api-google-calendar", "api-microsoft"],
        "connect_inbox" => &["api-google-gmail", "api-microsoft"],
        "connect_docs" => &["api-google-drive", "api-notion"],
        "connect_github" => &["api-github", "api-github-pat"],
        "connect_social" => &["api-twitter"],
        "connect_team_chat" => &["api-slack", "api-lark", "api-discord"],
        "watch_payments" => &["api-stripe"],
        _ => &[],
    }
}

fn scopes(id: &str, slug: &str) -> Vec<String> {
    // Google's granular catalog profiles already pin the appropriate scopes.
    // Microsoft defaults only to identity scopes; explicitly request the selected capability.
    match (id, slug) {
        ("connect_calendar", "api-microsoft") => vec!["Calendars.ReadWrite".into()],
        ("connect_inbox", "api-microsoft") => vec!["Mail.ReadWrite".into(), "Mail.Send".into()],
        _ => Vec::new(),
    }
}

pub async fn prepare(db: &Database, owner: &str, id: &str) -> AppResult<Setup> {
    let _rec = super::assistant_onboarding_service::CATALOG
        .iter()
        .find(|r| r.id == id)
        .filter(|r| !candidates(r.id).is_empty())
        .ok_or_else(|| AppError::NotFound("Suggestion not found".into()))?;
    let mut slugs = candidates(id).to_vec();
    if matches!(id, "connect_calendar" | "connect_inbox" | "connect_docs") {
        // A combined Google account already serves these intents, even with a custom proxy slug.
        slugs.extend(["api-google", "api-google-workspace"]);
    }
    let catalogs: Vec<_> = db
        .collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .find(doc! {"slug": {"$in": &slugs}, "is_active": true, "visibility": "public"})
        .await?
        .try_collect()
        .await?;
    let ids: Vec<_> = catalogs.iter().map(|s| s.id.as_str()).collect();
    // Hosted OAuth provisions a UserService before authorization completes.
    // Only its active, same-owner credential proves that setup is complete.
    let connected = db.collection::<Document>(crate::models::user_service::COLLECTION_NAME)
        .aggregate(vec![
            doc! {"$match": {"user_id": owner, "is_active": true, "catalog_service_id": {"$in": ids}}},
            doc! {"$lookup": {"from": crate::models::user_api_key::COLLECTION_NAME,
                "localField":"api_key_id", "foreignField":"_id", "as":"credential",
                "pipeline": [{"$match":{"user_id":owner,"status":"active"}}, {"$project":{"_id":1}}]}},
            doc! {"$match":{"credential.0":{"$exists":true}}},
            doc! {"$limit":1}, doc! {"$project":{"_id":1}},
        ]).await?.try_next().await?.is_some();
    let mut choices = Vec::new();
    if !connected {
        for slug in candidates(id) {
            let Some(service) = catalogs.iter().find(|s| s.slug == *slug) else {
                continue;
            };
            if service.service_type == "ssh" {
                continue;
            }
            let info = super::connect_link_service::catalog_info(db, service.clone()).await?;
            // OAuth without a configured client is a developer setup, not an onboarding choice.
            // Missing/inactive providers must not accidentally become generic API-key choices.
            if info.requires_gateway_url
                || (service.provider_config_id.is_some() && info.provider_id.is_none())
            {
                continue;
            }
            if !scopes(id, slug).is_empty() && !info.supports_oauth_scopes {
                continue;
            }
            let usable = match info.connect_method() {
                "oauth" | "device_code" if info.has_oauth_endpoints => {
                    let personal = if matches!(
                        info.credential_mode.as_deref(),
                        Some("user" | "both")
                    ) {
                        let mut filter = doc! {"user_id": owner, "provider_config_id": &info.provider_id,
                        "client_id_encrypted": {"$ne": mongodb::bson::Bson::Null}};
                        if info.connect_method() == "oauth" {
                            filter.insert(
                                "client_secret_encrypted",
                                doc! {"$ne": mongodb::bson::Bson::Null},
                            );
                        }
                        db.collection::<Document>(
                            crate::models::user_provider_credentials::COLLECTION_NAME,
                        )
                        .find_one(filter)
                        .projection(doc! {"_id": 1})
                        .await?
                        .is_some()
                    } else {
                        false
                    };
                    personal
                        || (info.credential_mode.as_deref() != Some("user")
                            && info.has_platform_oauth_credentials)
                }
                "api_key" => info.provider_type.as_deref() == Some("api_key"),
                _ => false,
            };
            if usable {
                choices.push(Choice {
                    slug: info.service_slug,
                    name: info.service_name,
                    scopes: scopes(id, slug),
                });
            }
        }
    }
    Ok(Setup {
        connected,
        choices,
        prompt: format!(
            "Use my existing connected account. {}",
            match id {
                "connect_calendar" => "Help me plan my day and schedule meetings.",
                "connect_inbox" => "Help me triage my inbox and draft replies.",
                "connect_docs" => "Help me find and work with my documents.",
                "connect_github" =>
                    "Create a Code specialist that can track my pull requests and issues.",
                "connect_social" => "Help me draft posts and track mentions on X.",
                "connect_team_chat" => "Help me search team conversations and post updates.",
                "watch_payments" => "Set up failed payment alerts and a weekly revenue summary.",
                _ => unreachable!("validated suggestion"),
            }
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        downstream_service::{COLLECTION_NAME as SERVICES, test_helpers::dummy_service},
        provider_config::COLLECTION_NAME as PROVIDERS,
    };
    use crate::services::connect_link_service as links;

    async fn fixture() -> (Database, String) {
        let db = crate::test_utils::connect_transaction_test_database("suggested_calendar").await;
        let owner = uuid::Uuid::new_v4().to_string();
        db.collection("users")
            .insert_one(crate::test_utils::test_user(
                &owner,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        for slug in ["api-google-calendar", "api-microsoft"] {
            let mut service = dummy_service();
            service.id = slug.into();
            service.slug = slug.into();
            service.name = if slug == "api-microsoft" {
                "Microsoft 365"
            } else {
                "Google Calendar"
            }
            .into();
            service.visibility = "public".into();
            service.provider_config_id = Some(slug.into());
            db.collection(SERVICES).insert_one(service).await.unwrap();
            db.collection(PROVIDERS).insert_one(doc! {
                "_id": slug, "slug": slug, "name": slug, "provider_type":"oauth2", "is_active":true,
                "authorization_url":"https://example.test/authorize", "token_url":"https://example.test/token",
                "client_id_encrypted": bson_bytes(), "client_secret_encrypted": bson_bytes(),
                "created_by":"system", "created_at":mongodb::bson::DateTime::now(), "updated_at":mongodb::bson::DateTime::now()
            }).await.unwrap();
        }
        (db, owner)
    }

    fn bson_bytes() -> mongodb::bson::Binary {
        mongodb::bson::Binary {
            subtype: mongodb::bson::spec::BinarySubtype::Generic,
            bytes: vec![1],
        }
    }

    #[tokio::test]
    async fn suggested_calendar_choices_create_real_hosted_links_with_calendar_scopes() {
        let (db, owner) = fixture().await;
        let setup = prepare(&db, &owner, "connect_calendar").await.unwrap();
        assert!(!setup.connected);
        assert_eq!(setup.choices.len(), 2);
        for choice in setup.choices {
            let link = links::create(
                &db,
                links::CreateInput {
                    user_id: owner.clone(),
                    service_slug: choice.slug.clone(),
                    scopes: choice.scopes,
                    use_platform_key: None,
                    assistant_intent_locked: false,
                    label: None,
                    endpoint_url: None,
                    requested_by: None,
                    callback_url: None,
                    ttl_secs: None,
                    oauth_client_id: None,
                },
            )
            .await
            .unwrap();
            let url = links::build_connect_url("https://nyxid.example", &link.raw_token).unwrap();
            assert!(url.starts_with("https://nyxid.example/connect/nyx_clk_"));
            let preview = links::preview(&db, &link.raw_token).await.unwrap();
            assert_eq!(preview.service.service_slug, choice.slug);
            if choice.slug == "api-microsoft" {
                assert_eq!(preview.link.scopes, ["Calendars.ReadWrite"]);
            }
        }
    }

    #[tokio::test]
    async fn suggested_calendar_excludes_unconfigured_inactive_and_nonpublic_providers() {
        let (db, owner) = fixture().await;
        db.collection::<Document>(PROVIDERS)
            .update_one(
                doc! {"_id":"api-microsoft"},
                doc! {"$unset":{"client_secret_encrypted":""}},
            )
            .await
            .unwrap();
        assert_eq!(
            prepare(&db, &owner, "connect_calendar")
                .await
                .unwrap()
                .choices
                .len(),
            1
        );
        db.collection::<Document>(PROVIDERS)
            .update_one(
                doc! {"_id":"api-google-calendar"},
                doc! {"$set":{"is_active":false}},
            )
            .await
            .unwrap();
        assert!(
            prepare(&db, &owner, "connect_calendar")
                .await
                .unwrap()
                .choices
                .is_empty()
        );
        db.collection::<Document>(PROVIDERS)
            .update_many(
                doc! {},
                doc! {"$set":{"is_active":true,"client_secret_encrypted":bson_bytes()}},
            )
            .await
            .unwrap();
        db.collection::<Document>(PROVIDERS)
            .update_one(
                doc! {"_id":"api-microsoft"},
                doc! {"$set":{"credential_mode":"user"}},
            )
            .await
            .unwrap();
        assert_eq!(
            prepare(&db, &owner, "connect_calendar")
                .await
                .unwrap()
                .choices
                .len(),
            1,
            "admin credentials do not bypass a personal-client-only provider"
        );
        db.collection("user_provider_credentials").insert_one(doc! {"_id":"personal-client", "user_id":&owner, "provider_config_id":"api-microsoft", "client_id_encrypted":bson_bytes(), "client_secret_encrypted":bson_bytes()}).await.unwrap();
        assert_eq!(
            prepare(&db, &owner, "connect_calendar")
                .await
                .unwrap()
                .choices
                .len(),
            2
        );
        db.collection::<Document>(PROVIDERS)
            .update_one(
                doc! {"_id":"api-microsoft"},
                doc! {"$unset":{"authorization_url":""}},
            )
            .await
            .unwrap();
        assert_eq!(
            prepare(&db, &owner, "connect_calendar")
                .await
                .unwrap()
                .choices
                .len(),
            1
        );
        db.collection::<Document>(SERVICES)
            .update_many(doc! {}, doc! {"$set":{"visibility":"private"}})
            .await
            .unwrap();
        assert!(
            prepare(&db, &owner, "connect_calendar")
                .await
                .unwrap()
                .choices
                .is_empty()
        );
    }

    #[tokio::test]
    async fn suggested_calendar_reuses_only_this_owners_active_catalog_binding() {
        let (db, owner) = fixture().await;
        db.collection("user_api_keys")
            .insert_one(doc! {"_id":"connected-key", "user_id":&owner, "status":"pending_auth"})
            .await
            .unwrap();
        db.collection("user_services").insert_one(doc! {"_id":"renamed", "user_id":"another-owner", "slug":"my-personal-calendar", "api_key_id":"connected-key", "catalog_service_id":"api-google-calendar", "is_active":true}).await.unwrap();
        assert!(
            !prepare(&db, &owner, "connect_calendar")
                .await
                .unwrap()
                .connected
        );
        db.collection::<Document>("user_services")
            .update_one(doc! {"_id":"renamed"}, doc! {"$set":{"user_id":&owner}})
            .await
            .unwrap();
        assert!(
            !prepare(&db, &owner, "connect_calendar")
                .await
                .unwrap()
                .connected,
            "OAuth is not complete yet"
        );
        db.collection::<Document>("user_api_keys")
            .update_one(
                doc! {"_id":"connected-key"},
                doc! {"$set":{"status":"active"}},
            )
            .await
            .unwrap();
        let setup = prepare(&db, &owner, "connect_calendar").await.unwrap();
        assert!(setup.connected);
        assert!(setup.choices.is_empty());
        assert!(setup.prompt.contains("Use my existing connected account"));
        db.collection::<Document>("user_services")
            .update_one(doc! {"_id":"renamed"}, doc! {"$set":{"is_active":false}})
            .await
            .unwrap();
        assert!(
            !prepare(&db, &owner, "connect_calendar")
                .await
                .unwrap()
                .connected
        );
    }

    #[test]
    fn all_connection_suggestions_use_choices_and_prompts_stay_small() {
        for rec in super::super::assistant_onboarding_service::CATALOG {
            if rec.id.starts_with("connect_") || rec.id == "watch_payments" {
                assert!(!candidates(rec.id).is_empty(), "{}", rec.id);
            }
            assert!(rec.prompt.len() + 150 < 4096);
        }
    }
}
