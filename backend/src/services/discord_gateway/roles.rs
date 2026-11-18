//! Authenticated Discord role identities only; names and message text are never authority.
use crate::models::channel_bot::ChannelBot;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
    time::{Duration, Instant},
};

const MAX_GUILDS: usize = 256;
const MAX_ENTRIES: usize = 2048;
const TTL: Duration = Duration::from_secs(300);
#[derive(Default)]
struct Cache(HashMap<(String, String), (Instant, Option<String>)>);
static CACHE: LazyLock<Mutex<Cache>> = LazyLock::new(Default::default);

fn identity(bot: &ChannelBot) -> String {
    format!("{}:{}", bot.id, bot.platform_bot_id)
}
fn own_role(role: &Value, bot: &str) -> Option<String> {
    (role["managed"] == true && role["tags"]["bot_id"] == bot)
        .then(|| {
            role["id"]
                .as_str()
                .filter(|id| super::facts::snowflake(id))
                .map(str::to_owned)
        })
        .flatten()
}
fn from_roles(roles: &[Value], bot: &str) -> Option<String> {
    let mut own = roles.iter().filter_map(|role| own_role(role, bot));
    let id = own.next();
    if own.next().is_some() { None } else { id }
}
impl Cache {
    fn put(&mut self, bot: &str, guild: &str, role: Option<String>) {
        self.0.retain(|_, (at, _)| at.elapsed() < TTL);
        let key = (bot.to_owned(), guild.to_owned());
        if !self.0.contains_key(&key) {
            let per_bot = self.0.keys().filter(|(id, _)| id == bot).count() >= MAX_GUILDS;
            if (per_bot || self.0.len() >= MAX_ENTRIES)
                && let Some(oldest) = self
                    .0
                    .iter()
                    .filter(|((id, _), _)| !per_bot || id == bot)
                    .min_by_key(|(_, (at, _))| *at)
                    .map(|(key, _)| key.clone())
            {
                self.0.remove(&oldest);
            }
        }
        self.0.insert(key, (Instant::now(), role));
    }
    fn get(&self, bot: &str, guild: &str) -> Option<Option<String>> {
        self.0
            .get(&(bot.into(), guild.into()))
            .filter(|(at, _)| at.elapsed() < TTL)
            .map(|(_, role)| role.clone())
    }
    fn observe(&mut self, bot: &ChannelBot, event: &Value) {
        let data = &event["d"];
        let name = event["t"].as_str().unwrap_or_default();
        if name == "READY" {
            self.0.retain(|(id, _), _| id != &identity(bot));
            return;
        }
        let guild = if matches!(name, "GUILD_CREATE" | "GUILD_DELETE") {
            &data["id"]
        } else {
            &data["guild_id"]
        };
        let Some(guild) = guild.as_str().filter(|id| super::facts::snowflake(id)) else {
            return;
        };
        match name {
            "GUILD_CREATE" => {
                if let Some(roles) = data["roles"].as_array() {
                    self.put(
                        &identity(bot),
                        guild,
                        from_roles(roles, &bot.platform_bot_id),
                    );
                }
            }
            "GUILD_DELETE" => {
                self.0.remove(&(identity(bot), guild.into()));
            }
            "GUILD_ROLE_CREATE" | "GUILD_ROLE_UPDATE" => {
                let role = &data["role"];
                if let Some(id) = own_role(role, &bot.platform_bot_id) {
                    self.put(&identity(bot), guild, Some(id));
                } else if self.get(&identity(bot), guild).flatten().as_deref()
                    == role["id"].as_str()
                {
                    self.put(&identity(bot), guild, None);
                }
            }
            "GUILD_ROLE_DELETE"
                if self.get(&identity(bot), guild).flatten().as_deref()
                    == data["role_id"].as_str() =>
            {
                self.put(&identity(bot), guild, None);
            }
            _ => {}
        }
    }
}

/// Keep just the identity projection in the socket dispatch queue.
pub(super) fn project(event: &Value, bot: &str) -> Value {
    let d = &event["d"];
    let role = |r: &Value| json!({"id":r["id"],"managed":r["managed"],"tags":{"bot_id":r["tags"]["bot_id"]}});
    let data = match event["t"].as_str() {
        Some("GUILD_CREATE") => {
            json!({"id":d["id"],"roles":d["roles"].as_array().map(|roles|roles.iter()
            .filter(|r|own_role(r,bot).is_some()).take(2).map(role).collect::<Vec<_>>())})
        }
        Some("GUILD_ROLE_CREATE" | "GUILD_ROLE_UPDATE") => {
            json!({"guild_id":d["guild_id"],"role":role(&d["role"])})
        }
        Some("GUILD_ROLE_DELETE") => json!({"guild_id":d["guild_id"],"role_id":d["role_id"]}),
        Some("GUILD_DELETE") => json!({"id":d["id"]}),
        _ => Value::Null,
    };
    json!({"t":event["t"],"s":event["s"],"d":data})
}
pub(crate) fn observe(bot: &ChannelBot, event: &Value) {
    CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .observe(bot, event);
}
pub(crate) fn mentioned(bot: &ChannelBot, message: &Value) -> bool {
    let Some(guild) = message["guild_id"].as_str() else {
        return false;
    };
    let role = CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&identity(bot), guild)
        .flatten();
    role.is_some_and(|role| {
        message["mention_roles"]
            .as_array()
            .is_some_and(|roles| roles.iter().any(|r| r.as_str() == Some(&role)))
    })
}

/// Bounded authenticated lookup only on a role-mention cache miss (including a
/// resumed socket after process restart and native intake without a socket).
pub(crate) async fn ensure(bot: &ChannelBot, token: &str, message: &Value) {
    if message["mention_roles"]
        .as_array()
        .is_none_or(|roles| roles.is_empty() || roles.len() > 256)
    {
        return;
    }
    let Some(guild) = message["guild_id"]
        .as_str()
        .filter(|id| super::facts::snowflake(id))
    else {
        return;
    };
    if CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&identity(bot), guild)
        .is_some()
    {
        return;
    }
    let result = crate::services::nyxbot_gateway_source::discord_read(
        bot,
        token,
        &format!("/guilds/{guild}/roles"),
    )
    .await;
    if let Ok(value) = result
        && let Some(roles) = value.as_array().filter(|roles| roles.len() <= 512)
    {
        // Do not overwrite a newer socket observation while this read waited.
        let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if cache.get(&identity(bot), guild).is_none() {
            cache.put(
                &identity(bot),
                guild,
                from_roles(roles, &bot.platform_bot_id),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn role_cache_is_scoped_bounded_and_updates_without_names() {
        let bot = super::super::tests::bot();
        let mut cache = Cache::default();
        let own = json!({"id":"900","managed":true,"tags":{"bot_id":"100"},"name":"private name"});
        let other = json!({"id":"901","managed":true,"tags":{"bot_id":"999"},"name":"same name"});
        let event =
            json!({"t":"GUILD_CREATE","d":{"id":"300","roles":[own.clone(),other.clone()]}});
        let projected = project(&event, "100");
        assert!(!projected.to_string().contains("name"));
        cache.observe(&bot, &projected);
        assert_eq!(cache.get(&identity(&bot), "300"), Some(Some("900".into())));
        assert_eq!(cache.get("other-bot", "300"), None);
        cache.observe(
            &bot,
            &json!({"t":"GUILD_ROLE_UPDATE","d":{"guild_id":"300","role":other}}),
        );
        assert_eq!(cache.get(&identity(&bot), "300"), Some(Some("900".into())));
        cache.observe(
            &bot,
            &json!({"t":"GUILD_ROLE_DELETE","d":{"guild_id":"300","role_id":"900"}}),
        );
        assert_eq!(cache.get(&identity(&bot), "300"), Some(None));
        cache.observe(
            &bot,
            &json!({"t":"GUILD_ROLE_CREATE","d":{"guild_id":"300","role":own}}),
        );
        cache.observe(&bot,&json!({"t":"GUILD_ROLE_UPDATE","d":{"guild_id":"300","role":{"id":"900","managed":true,"tags":{"bot_id":"999"}}}}));
        assert_eq!(cache.get(&identity(&bot), "300"), Some(None));
        for guild in 1..=MAX_GUILDS + 1 {
            cache.put(&identity(&bot), &guild.to_string(), None);
        }
        assert_eq!(cache.0.len(), MAX_GUILDS);
        cache.observe(&bot, &json!({"t":"READY"}));
        assert!(cache.0.is_empty());
        for n in 0..MAX_ENTRIES + 1 {
            cache.put(&n.to_string(), "300", None);
        }
        assert_eq!(cache.0.len(), MAX_ENTRIES);
    }

    #[tokio::test]
    async fn role_lookup_and_normalization_use_only_authenticated_own_identity() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{header, path},
        };
        let bot = super::super::tests::bot();
        let http = MockServer::start().await;
        crate::services::nyxbot_gateway_source::TEST_DISCORD_BASES
            .lock()
            .unwrap()
            .insert(bot.id.clone(), http.uri());
        Mock::given(path("/guilds/300/roles"))
            .and(header("authorization", "Bot token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                {"id":"900","name":"same name","managed":true,"tags":{"bot_id":"100"}},
                {"id":"901","name":"same name","managed":true,"tags":{"bot_id":"999"}}
            ])))
            .expect(1)
            .mount(&http)
            .await;
        let mut message = json!({"id":"500","type":0,"channel_id":"200","guild_id":"300","author":{"id":"7"},
            "content":"private content","mentions":[],"mention_roles":["900"],"mention_everyone":false});
        for _ in 0..2 {
            ensure(&bot, "token", &message).await;
        }
        let channel = json!({"id":"200","type":0,"guild_id":"300"});
        let facts = super::super::facts::authenticated_facts(&bot, &json!({"d":message}), &channel)
            .unwrap();
        assert_eq!(
            facts.address,
            crate::models::channel_thread::ThreadAddress::Mention
        );
        assert!(
            facts
                .discord_gateway
                .as_ref()
                .unwrap()
                .managed_role_mentioned
        );
        assert_eq!(
            facts.discord_gateway.as_ref().unwrap().broadcast_role_count,
            0
        );
        assert!(
            !serde_json::to_string(&facts)
                .unwrap()
                .contains("private content")
        );
        for roles in [json!(["901"]), json!([])] {
            message["mention_roles"] = roles;
            message["mention_everyone"] = json!(true);
            let facts =
                super::super::facts::authenticated_facts(&bot, &json!({"d":message}), &channel)
                    .unwrap();
            assert_eq!(
                facts.address,
                crate::models::channel_thread::ThreadAddress::NotAddressed
            );
        }
        let mut legacy = serde_json::to_value(facts.discord_gateway.unwrap()).unwrap();
        legacy
            .as_object_mut()
            .unwrap()
            .remove("managed_role_mentioned");
        legacy
            .as_object_mut()
            .unwrap()
            .remove("broadcast_role_count");
        let legacy: crate::models::discord_gateway::DiscordGatewayFacts =
            serde_json::from_value(legacy).unwrap();
        assert!(!legacy.managed_role_mentioned);
        crate::services::nyxbot_gateway_source::TEST_DISCORD_BASES
            .lock()
            .unwrap()
            .remove(&bot.id);
    }
}
