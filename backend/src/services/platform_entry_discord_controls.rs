//! The shared entry owns one DM CHAT_INPUT command, never the application's
//! whole command set. Provisioning is called only by explicit admin opt-in.
use crate::errors::{AppError, AppResult};
use serde_json::{Value, json};

pub(super) const DISCORD_BASE: &str = "https://discord.com/api/v10";
const RESPONSE_BYTES: usize = 64 * 1024;

pub(crate) fn schema() -> Value {
    json!({
        "type":1,"name":"nyx","description":"NyxID public help and your personal NyxBot",
        "contexts":[1],"integration_types":[0],
        "nsfw":false,"default_member_permissions":null,"dm_permission":true,
        "options":[
            {"type":1,"name":"login","description":"Review or create your NyxID account link"},
            {"type":1,"name":"status","description":"Check this private chat's login state"},
            {"type":1,"name":"logout","description":"Disconnect this chat and cancel pending login proofs"},
            {"type":1,"name":"help","description":"Show chat controls and public help"},
            {"type":1,"name":"confirm","description":"Confirm the account reviewed on NyxID","options":[
                {"type":3,"name":"code","description":"The P- confirmation code from your account review","required":true,"min_length":11,"max_length":32}
            ]},
            {"type":1,"name":"ask","description":"Ask a question or reply to your personal action card","options":[
                {"type":3,"name":"question","description":"Your message, including an exact action-card reply when needed","required":true,"min_length":1,"max_length":1024}
            ]}
        ]
    })
}

pub(crate) fn schema_digest() -> String {
    super::mcp_service::canonical_sha256(schema())
}

pub(crate) struct InstalledCommand {
    pub(crate) application_id: String,
    pub(crate) command_id: String,
    pub(crate) command_digest: String,
}

pub(super) fn unavailable() -> AppError {
    AppError::Conflict(
        "Discord private controls could not be verified; entry remains disabled".into(),
    )
}

fn option_projection(value: &Value) -> AppResult<Value> {
    let source = value.as_object().ok_or_else(unavailable)?;
    let mut target = serde_json::Map::new();
    for field in ["type", "name", "description"] {
        target.insert(
            field.into(),
            source.get(field).cloned().ok_or_else(unavailable)?,
        );
    }
    // Omitted false values and empty child lists have the same command meaning.
    // Localization/GET decorations do not change the canonical option keys.
    for field in ["required", "autocomplete"] {
        if let Some(value) = source.get(field).filter(|value| !value.is_null()) {
            if !value.is_boolean() {
                return Err(unavailable());
            }
            if value == &Value::Bool(true) {
                target.insert(field.into(), value.clone());
            }
        }
    }
    for field in [
        "min_value",
        "max_value",
        "min_length",
        "max_length",
        "channel_types",
    ] {
        if let Some(value) = source.get(field).filter(|value| !value.is_null()) {
            target.insert(field.into(), value.clone());
        }
    }
    if let Some(children) = source.get("options").filter(|value| !value.is_null()) {
        let children = children.as_array().ok_or_else(unavailable)?;
        if !children.is_empty() {
            target.insert(
                "options".into(),
                Value::Array(
                    children
                        .iter()
                        .map(option_projection)
                        .collect::<AppResult<_>>()?,
                ),
            );
        }
    }
    if let Some(choices) = source.get("choices").filter(|value| !value.is_null()) {
        let choices = choices.as_array().ok_or_else(unavailable)?;
        if !choices.is_empty() {
            let choices = choices
                .iter()
                .map(|choice| {
                    Ok(json!({"name": choice.get("name").ok_or_else(unavailable)?,
                    "value": choice.get("value").ok_or_else(unavailable)?}))
                })
                .collect::<AppResult<Vec<_>>>()?;
            target.insert("choices".into(), Value::Array(choices));
        }
    }
    for field in source.keys() {
        if ![
            "type",
            "name",
            "description",
            "required",
            "autocomplete",
            "min_value",
            "max_value",
            "min_length",
            "max_length",
            "channel_types",
            "options",
            "choices",
            "name_localizations",
            "description_localizations",
            "name_localized",
            "description_localized",
        ]
        .contains(&field.as_str())
        {
            return Err(unavailable());
        }
    }
    Ok(Value::Object(target))
}

pub(super) fn projection(value: &Value) -> AppResult<Value> {
    let map = value.as_object().ok_or_else(unavailable)?;
    let mut projected = serde_json::Map::new();
    for field in [
        "type",
        "name",
        "description",
        "contexts",
        "integration_types",
    ] {
        projected.insert(
            field.into(),
            map.get(field).cloned().ok_or_else(unavailable)?,
        );
    }
    projected.insert(
        "options".into(),
        Value::Array(
            map.get("options")
                .and_then(Value::as_array)
                .ok_or_else(unavailable)?
                .iter()
                .map(option_projection)
                .collect::<AppResult<_>>()?,
        ),
    );
    for (field, default) in [
        ("nsfw", json!(false)),
        ("default_member_permissions", Value::Null),
        ("dm_permission", json!(true)),
    ] {
        projected.insert(field.into(), map.get(field).cloned().unwrap_or(default));
    }
    Ok(Value::Object(projected))
}

pub(super) async fn response(request: reqwest::RequestBuilder) -> AppResult<Value> {
    response_until(request, None).await
}
async fn response_until(
    request: reqwest::RequestBuilder,
    deadline: Option<chrono::DateTime<chrono::Utc>>,
) -> AppResult<Value> {
    use futures::StreamExt;
    let remaining = match deadline {
        Some(deadline) => (deadline - chrono::Utc::now())
            .to_std()
            .ok()
            .filter(|duration| !duration.is_zero())
            .ok_or_else(unavailable)?,
        None => std::time::Duration::from_secs(20),
    };
    let end = tokio::time::Instant::now() + remaining.min(std::time::Duration::from_secs(20));
    tokio::time::timeout_at(end, async move {
        // Checked inside the polled future too: a task paused before polling
        // cannot start a provider write after its fixed dispatch authorization.
        if deadline.is_some_and(|deadline| chrono::Utc::now() >= deadline) {
            return Err(unavailable());
        }
        let response = request.send().await.map_err(|_| unavailable())?;
        if !response.status().is_success()
            || response
                .content_length()
                .is_some_and(|size| size > RESPONSE_BYTES as u64)
        {
            return Err(unavailable());
        }
        let mut stream = response.bytes_stream();
        let mut bytes = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| unavailable())?;
            if bytes
                .len()
                .checked_add(chunk.len())
                .is_none_or(|size| size > RESPONSE_BYTES)
            {
                return Err(unavailable());
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| unavailable())
    })
    .await
    .map_err(|_| unavailable())?
}

#[cfg(test)]
async fn install_at(
    http: &reqwest::Client,
    token: &str,
    expected_bot_id: &str,
    expected_public_key: &str,
    owned_command_id: Option<&str>,
    base: &str,
) -> AppResult<InstalledCommand> {
    use super::platform_entry_identity::opaque;
    let application = response(
        http.get(format!("{base}/oauth2/applications/@me"))
            .header("authorization", format!("Bot {token}")),
    )
    .await?;
    let app = opaque(application["id"].as_str().ok_or_else(unavailable)?)?;
    if application["bot"]["id"].as_str() != Some(expected_bot_id) {
        return Err(unavailable());
    }
    // User-entered keys cannot authenticate a shared sender. This key comes
    // from the provider under the exact bot token and is fenced with the
    // encrypted credential snapshot before the entry can become enabled.
    let verification_key = application["verify_key"].as_str().ok_or_else(unavailable)?;
    let provider_key = hex::decode(verification_key).map_err(|_| unavailable())?;
    let stored_key = hex::decode(expected_public_key).map_err(|_| unavailable())?;
    if provider_key.len() != 32
        || stored_key.len() != 32
        || !crate::crypto::token::constant_time_eq(&provider_key, &stored_key)
    {
        return Err(unavailable());
    }
    // IDs select fixed provider paths. Discord's actual REST identifiers are
    // numeric strings; do not coerce them into Telegram or machine integers.
    if !app.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(unavailable());
    }
    let url = format!("{base}/applications/{app}/commands");
    let commands = response(
        http.get(&url)
            .header("authorization", format!("Bot {token}")),
    )
    .await?;
    let commands = commands.as_array().ok_or_else(unavailable)?;
    mutate_verified_at(
        http,
        token,
        app,
        commands,
        owned_command_id,
        base,
        chrono::Utc::now() + chrono::Duration::seconds(20),
    )
    .await
}

/// Caller has authenticated the application/key and durably claimed the exact
/// observed namespace before this one mutation. No pre-dispatch network reads.
pub(super) async fn mutate_verified_at(
    http: &reqwest::Client,
    token: &str,
    app: &str,
    commands: &[Value],
    owned_command_id: Option<&str>,
    base: &str,
    deadline: chrono::DateTime<chrono::Utc>,
) -> AppResult<InstalledCommand> {
    use super::platform_entry_identity::opaque;
    if chrono::Utc::now() >= deadline {
        return Err(unavailable());
    }
    let url = format!("{base}/applications/{app}/commands");
    let matching = commands
        .iter()
        .filter(|command| command["type"] == 1 && command["name"] == "nyx")
        .collect::<Vec<_>>();
    if matching.len() > 1 {
        return Err(unavailable());
    }
    let result = match (matching.first(), owned_command_id) {
        (Some(command), Some(owned)) if command["id"].as_str() == Some(owned) => {
            if projection(command)? == schema() {
                (*command).clone()
            } else {
                if !owned.bytes().all(|byte| byte.is_ascii_digit()) {
                    return Err(unavailable());
                }
                response_until(
                    http.patch(format!("{url}/{owned}"))
                        .header("authorization", format!("Bot {token}"))
                        .json(&schema()),
                    Some(deadline),
                )
                .await?
            }
        }
        (None, None) => {
            response_until(
                http.post(&url)
                    .header("authorization", format!("Bot {token}"))
                    .json(&schema()),
                Some(deadline),
            )
            .await?
        }
        // A pre-existing unowned command is never adopted or overwritten.
        // A vanished owned command also requires a new explicit setup decision.
        _ => return Err(unavailable()),
    };
    if result["application_id"].as_str() != Some(app) || projection(&result)? != schema() {
        return Err(unavailable());
    }
    let id = opaque(result["id"].as_str().ok_or_else(unavailable)?)?;
    if !id.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(unavailable());
    }
    if let Some(owned) = owned_command_id
        && id != owned
    {
        return Err(unavailable());
    }
    Ok(InstalledCommand {
        application_id: app.into(),
        command_id: id.into(),
        command_digest: schema_digest(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, method, path},
    };

    async fn application(server: &MockServer) {
        Mock::given(method("GET"))
            .and(path("/oauth2/applications/@me"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(
                    json!({"id":"100","bot":{"id":"200"},"verify_key":"a1".repeat(32)}),
                ),
            )
            .expect(1)
            .mount(server)
            .await;
    }
    fn registered() -> Value {
        let mut value = schema();
        value["id"] = json!("300");
        value["application_id"] = json!("100");
        value["version"] = json!("400");
        value["name_localizations"] = Value::Null;
        value["description_localizations"] = Value::Null;
        value["name_localized"] = json!("nyx");
        fn decorations(options: &mut Value) {
            for option in options.as_array_mut().unwrap() {
                option["name_localizations"] = Value::Null;
                option["description_localizations"] = Value::Null;
                option["name_localized"] = option["name"].clone();
                option["description_localized"] = option["description"].clone();
                option["autocomplete"] = json!(false);
                if option.get("required").is_none() {
                    option["required"] = json!(false);
                }
                if let Some(children) = option.get_mut("options") {
                    decorations(children);
                }
            }
        }
        decorations(&mut value["options"]);
        value
    }

    #[test]
    fn provider_decorations_and_defaults_preserve_recursive_option_meaning() {
        let response = registered();
        assert_eq!(projection(&response).unwrap(), schema());
        for mutation in 0..6 {
            let mut changed = response.clone();
            match mutation {
                0 => changed["options"][4]["options"][0]["required"] = json!(false),
                1 => changed["options"][4]["options"][0]["max_length"] = json!(2000),
                2 => changed["options"][5]["options"][0]["name"] = json!("other"),
                3 => changed["options"][5]["options"][0]["autocomplete"] = json!(true),
                4 => changed["default_member_permissions"] = json!("8"),
                _ => changed["dm_permission"] = json!(false),
            }
            assert_ne!(
                projection(&changed).unwrap(),
                schema(),
                "semantic mutation {mutation}"
            );
        }
    }

    #[tokio::test]
    async fn creates_only_owned_dm_command_and_preserves_unrelated_commands() {
        let server = MockServer::start().await;
        application(&server).await;
        Mock::given(method("GET"))
            .and(path("/applications/100/commands"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!([{"id":"999","name":"other","type":1}])),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/applications/100/commands"))
            .and(body_json(schema()))
            .respond_with(ResponseTemplate::new(201).set_body_json(registered()))
            .expect(1)
            .mount(&server)
            .await;
        let installed = install_at(
            &reqwest::Client::new(),
            "synthetic-bot-token",
            "200",
            &"a1".repeat(32),
            None,
            &server.uri(),
        )
        .await
        .unwrap();
        assert_eq!(installed.command_id, "300");
        assert_eq!(installed.application_id, "100");
        assert_eq!(installed.command_digest, schema_digest());
        assert_eq!(server.received_requests().await.unwrap().len(), 3);
    }

    #[tokio::test]
    async fn conflicting_preexisting_nyx_is_refused_without_a_mutation() {
        let server = MockServer::start().await;
        application(&server).await;
        Mock::given(method("GET"))
            .and(path("/applications/100/commands"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!([{"id":"999","name":"nyx","type":1}])),
            )
            .expect(1)
            .mount(&server)
            .await;
        assert!(
            install_at(
                &reqwest::Client::new(),
                "synthetic-bot-token",
                "200",
                &"a1".repeat(32),
                None,
                &server.uri()
            )
            .await
            .is_err()
        );
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests.iter().all(|request| request.method == "GET"));
    }

    #[tokio::test]
    async fn update_targets_exact_owned_command_and_never_replaces_command_collection() {
        let server = MockServer::start().await;
        application(&server).await;
        let mut prior = registered();
        prior["nsfw"] = json!(true);
        Mock::given(method("GET"))
            .and(path("/applications/100/commands"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!([prior,{"id":"999","name":"other","type":1}])),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("PATCH"))
            .and(path("/applications/100/commands/300"))
            .and(body_json(schema()))
            .respond_with(ResponseTemplate::new(200).set_body_json(registered()))
            .expect(1)
            .mount(&server)
            .await;
        install_at(
            &reqwest::Client::new(),
            "synthetic-bot-token",
            "200",
            &"a1".repeat(32),
            Some("300"),
            &server.uri(),
        )
        .await
        .unwrap();
        assert_eq!(server.received_requests().await.unwrap().len(), 3);
    }
    #[tokio::test]
    async fn correct_token_bot_and_app_cannot_bless_an_arbitrary_signature_key() {
        let server = MockServer::start().await;
        application(&server).await;
        assert!(
            install_at(
                &reqwest::Client::new(),
                "synthetic-bot-token",
                "200",
                &"b2".repeat(32),
                None,
                &server.uri()
            )
            .await
            .is_err()
        );
        let requests = server.received_requests().await.unwrap();
        assert_eq!(
            requests.len(),
            1,
            "mismatched key cannot list/create commands or enable entry"
        );
    }
}
