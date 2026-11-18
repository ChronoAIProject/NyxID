//! Browser destinations shared by assistant tools, setup APIs and notifications.

pub const AGENT_DETAILS_GRANTS: &str = "grants";
pub const AGENT_DETAILS_SKILLS: &str = "skills";
/// The agent's Chats settings (channel bots and their chats).
pub const AGENT_DETAILS_CHANNELS: &str = "channels";

pub const NYXBOT_SETTINGS_SECTIONS: &[&str] = &[
    "permissions",
    "tasks",
    "automations",
    "notifications",
    "channels",
    "channels/adapters",
    "channels/links",
];

pub const AGENT_DETAILS_SECTIONS: &[&str] = &[
    "overview",
    "overview/persona",
    "overview/voice",
    "overview/automations",
    "grants",
    "grants/services",
    "grants/machines",
    "grants/connections",
    "skills",
    "memory",
    "channels",
];

pub const ACCOUNT_SETTINGS_SECTIONS: &[&str] = &[
    "profile",
    "security",
    "security/delete",
    "sessions",
    "mcp",
    "display",
    "display/layout",
    "display/colors",
    "privacy",
];

pub const BILLING_SECTIONS: &[&str] = &[
    "billing",
    "billing/topup",
    "billing/benefits",
    "billing/bill",
    "billing/machines",
    "billing/activity",
    "usage",
    "usage/charts",
    "usage/explorer",
];

#[derive(Clone, Copy)]
pub enum AssistantPage<'a> {
    Agent {
        agent: Option<&'a str>,
    },
    TelegramSetup {
        request: Option<&'a str>,
    },
    Group {
        group: &'a str,
    },
    Approvals,
    ApprovalSettings,
    Conversation {
        conversation: &'a str,
    },
    Settings {
        tab: &'a str,
    },
    NyxBotSettings {
        section: Option<&'a str>,
    },
    ReviewCard {
        conversation: &'a str,
        card: &'a str,
    },
    Billing {
        tab: &'a str,
        section: Option<&'a str>,
    },
    /// An agent's details sheet, opened on a tab or slash-separated section.
    /// The frontend normalizes unknown paths to their nearest known ancestor.
    AgentDetails {
        agent: &'a str,
        tab: &'a str,
    },
    Automations {
        setup: Option<&'a str>,
        agent: Option<&'a str>,
    },
    Machines,
    MachineSettings {
        node: &'a str,
    },
    SavedLogins,
    MachineSetup {
        setup: &'a str,
    },
    MachinePair {
        code: &'a str,
    },
    MachineDesktop {
        node: &'a str,
        conversation: Option<&'a str>,
        context_id: Option<&'a str>,
        display: nyxid_machine::desktop::Display,
    },
}

impl AssistantPage<'_> {
    pub fn path(self) -> String {
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        let path = match self {
            Self::Agent { agent } => {
                if let Some(agent) = agent {
                    query.append_pair("agent", agent);
                }
                "/assistant".into()
            }
            Self::TelegramSetup { request } => {
                query.append_pair("connect", "telegram-new");
                if let Some(request) = request {
                    query.append_pair("request_id", request);
                }
                "/channel-bots".into()
            }
            Self::Group { group } => {
                query.append_pair("c", group);
                "/assistant".into()
            }
            Self::Approvals => "/assistant/approvals".into(),
            Self::ApprovalSettings => "/approvals/settings".into(),
            Self::Conversation { conversation } => {
                query.append_pair("c", conversation);
                "/assistant".into()
            }
            Self::Settings { tab } => {
                query
                    .append_pair("panel", "settings")
                    .append_pair("panelTab", tab);
                "/assistant".into()
            }
            Self::ReviewCard { conversation, card } => {
                query
                    .append_pair("c", conversation)
                    .append_pair("card", card);
                "/assistant".into()
            }
            Self::NyxBotSettings { section } => {
                query.append_pair("panel", "nyxbot");
                if let Some(section) = section {
                    query.append_pair("panelTab", section);
                }
                "/assistant".into()
            }
            Self::Billing { tab, section } => {
                query
                    .append_pair("panel", "billing")
                    .append_pair("panelTab", tab);
                if let Some(section) = section {
                    query.append_pair("panelSection", section);
                }
                "/assistant".into()
            }
            Self::AgentDetails { agent, tab } => {
                query
                    .append_pair("agent", agent)
                    .append_pair("details", tab);
                "/assistant".into()
            }
            Self::Automations { setup, agent } => {
                if let Some(setup) = setup {
                    query.append_pair("setup", setup);
                }
                if let Some(agent) = agent {
                    query.append_pair("agent", agent);
                }
                "/assistant/automations".into()
            }
            Self::Machines => "/assistant/machines".into(),
            Self::MachineSettings { node } => {
                query.append_pair("machine", node);
                "/assistant/machines".into()
            }
            Self::SavedLogins => {
                query.append_pair("tab", "logins");
                "/assistant/machines".into()
            }
            Self::MachineSetup { setup } => {
                query.append_pair("setup", setup);
                "/assistant/machines/new".into()
            }
            Self::MachinePair { code } => {
                query.append_pair("code", code);
                "/assistant/machines/pair".into()
            }
            Self::MachineDesktop {
                node,
                conversation,
                context_id,
                display,
            } => {
                if let Some(conversation) = conversation {
                    query.append_pair("conversation_id", conversation);
                }
                if let Some(context_id) = context_id {
                    query.append_pair("context_id", context_id);
                }
                if display == nyxid_machine::desktop::Display::Dev {
                    query.append_pair("display", "dev");
                }
                format!(
                    "/assistant/machines/{}/desktop",
                    url::form_urlencoded::byte_serialize(node.as_bytes()).collect::<String>()
                )
            }
        };
        let query = query.finish();
        if query.is_empty() {
            path
        } else {
            format!("{path}?{query}")
        }
    }

    pub fn url(self, frontend_url: &str) -> String {
        format!("{}{}", frontend_url.trim_end_matches('/'), self.path())
    }
}

#[cfg(test)]
mod tests {
    use super::AssistantPage::*;
    use super::{AGENT_DETAILS_GRANTS, AGENT_DETAILS_SKILLS};

    #[test]
    fn browser_links_use_assistant_workspace_and_encode_parameters() {
        for (page, path) in [
            (
                MachineDesktop {
                    node: "a/b+% 中文",
                    conversation: Some("c+% &/中文"),
                    context_id: Some("x+% &/中文"),
                    display: nyxid_machine::desktop::Display::Dev,
                },
                "/assistant/machines/a%2Fb%2B%25+%E4%B8%AD%E6%96%87/desktop?conversation_id=c%2B%25+%26%2F%E4%B8%AD%E6%96%87&context_id=x%2B%25+%26%2F%E4%B8%AD%E6%96%87&display=dev",
            ),
            (
                Settings { tab: "profile" },
                "/assistant?panel=settings&panelTab=profile",
            ),
            (
                Settings { tab: "security" },
                "/assistant?panel=settings&panelTab=security",
            ),
            (
                Settings { tab: "sessions" },
                "/assistant?panel=settings&panelTab=sessions",
            ),
            (
                Settings { tab: "mcp" },
                "/assistant?panel=settings&panelTab=mcp",
            ),
            (
                Settings { tab: "display" },
                "/assistant?panel=settings&panelTab=display",
            ),
            (
                Settings { tab: "privacy" },
                "/assistant?panel=settings&panelTab=privacy",
            ),
            (
                ReviewCard {
                    conversation: "nyxa-test",
                    card: "a&b",
                },
                "/assistant?c=nyxa-test&card=a%26b",
            ),
            (Agent { agent: None }, "/assistant"),
            (Agent { agent: Some("a&b") }, "/assistant?agent=a%26b"),
            (
                TelegramSetup { request: None },
                "/channel-bots?connect=telegram-new",
            ),
            (
                TelegramSetup {
                    request: Some("a&b"),
                },
                "/channel-bots?connect=telegram-new&request_id=a%26b",
            ),
            (NyxBotSettings { section: None }, "/assistant?panel=nyxbot"),
            (Approvals, "/assistant/approvals"),
            (
                Billing {
                    tab: "billing",
                    section: None,
                },
                "/assistant?panel=billing&panelTab=billing",
            ),
            (
                Billing {
                    tab: "usage",
                    section: None,
                },
                "/assistant?panel=billing&panelTab=usage",
            ),
            (
                Settings {
                    tab: "a&b c/中文+%",
                },
                "/assistant?panel=settings&panelTab=a%26b+c%2F%E4%B8%AD%E6%96%87%2B%25",
            ),
            (
                Billing {
                    tab: "a&b c/中文+%",
                    section: None,
                },
                "/assistant?panel=billing&panelTab=a%26b+c%2F%E4%B8%AD%E6%96%87%2B%25",
            ),
            (
                Automations {
                    setup: None,
                    agent: None,
                },
                "/assistant/automations",
            ),
            (
                Automations {
                    setup: Some("a&b"),
                    agent: None,
                },
                "/assistant/automations?setup=a%26b",
            ),
            (
                Automations {
                    setup: None,
                    agent: Some("agent/中文 &"),
                },
                "/assistant/automations?agent=agent%2F%E4%B8%AD%E6%96%87+%26",
            ),
            (
                Automations {
                    setup: Some("s"),
                    agent: Some("a1"),
                },
                "/assistant/automations?setup=s&agent=a1",
            ),
            (
                AgentDetails {
                    agent: "a1",
                    tab: AGENT_DETAILS_GRANTS,
                },
                "/assistant?agent=a1&details=grants",
            ),
            (
                AgentDetails {
                    agent: "a&b/中文",
                    tab: AGENT_DETAILS_SKILLS,
                },
                "/assistant?agent=a%26b%2F%E4%B8%AD%E6%96%87&details=skills",
            ),
            (Machines, "/assistant/machines"),
            (SavedLogins, "/assistant/machines?tab=logins"),
            (
                MachineSetup { setup: "s" },
                "/assistant/machines/new?setup=s",
            ),
            (
                MachinePair { code: "AB CD" },
                "/assistant/machines/pair?code=AB+CD",
            ),
            (
                MachineDesktop {
                    node: "n",
                    conversation: None,
                    context_id: None,
                    display: nyxid_machine::desktop::Display::Dev,
                },
                "/assistant/machines/n/desktop?display=dev",
            ),
            (
                MachineDesktop {
                    node: "n",
                    display: nyxid_machine::desktop::Display::Secure,
                    conversation: Some("nyxagent:c"),
                    context_id: None,
                },
                "/assistant/machines/n/desktop?conversation_id=nyxagent%3Ac",
            ),
            (
                MachineDesktop {
                    node: "n",
                    display: nyxid_machine::desktop::Display::Secure,
                    conversation: None,
                    context_id: None,
                },
                "/assistant/machines/n/desktop",
            ),
        ] {
            assert_eq!(page.path(), path);
            assert_eq!(
                page.url("https://nyxid.test/"),
                format!("https://nyxid.test{path}")
            );
        }
    }
}

#[cfg(test)]
mod section_tests {
    use super::*;
    #[test]
    fn nyxbot_sections_match_frontend_and_encode_links() {
        let source =
            include_str!("../../../frontend/src/lib/assistant/nyxbot-settings-sections.ts");
        let list = source
            .split("export const NYXBOT_SETTINGS_SECTIONS = [")
            .nth(1)
            .unwrap()
            .split("] as const;")
            .next()
            .unwrap();
        let frontend: Vec<&str> = list
            .lines()
            .filter_map(|line| {
                line.trim()
                    .strip_prefix('"')
                    .and_then(|v| v.strip_suffix("\","))
            })
            .collect();
        assert_eq!(NYXBOT_SETTINGS_SECTIONS, frontend);
        for section in NYXBOT_SETTINGS_SECTIONS {
            let path = AssistantPage::NyxBotSettings {
                section: Some(section),
            }
            .path();
            let url = url::Url::parse(&format!("https://nyxid.test{path}")).unwrap();
            assert_eq!(
                url.query_pairs().find(|(k, _)| k == "panelTab").unwrap().1,
                *section
            );
        }
    }
}

#[cfg(test)]
mod clarity_section_tests {
    use super::*;
    fn frontend_sections(source: &str, name: &str) -> Vec<String> {
        source
            .split(&format!("export const {name} = ["))
            .nth(1)
            .unwrap()
            .split("] as const;")
            .next()
            .unwrap()
            .split(',')
            .filter_map(|value| {
                value
                    .trim()
                    .strip_prefix('"')
                    .and_then(|s| s.strip_suffix('"'))
                    .map(str::to_owned)
            })
            .collect()
    }
    #[test]
    fn settings_section_vocabularies_match_frontend() {
        assert_eq!(
            AGENT_DETAILS_SECTIONS,
            frontend_sections(
                include_str!("../../../frontend/src/lib/assistant/agent-details-sections.ts"),
                "AGENT_DETAILS_SECTIONS"
            )
        );
        assert_eq!(
            ACCOUNT_SETTINGS_SECTIONS,
            frontend_sections(
                include_str!("../../../frontend/src/lib/account-settings-sections.ts"),
                "ACCOUNT_SETTINGS_SECTIONS"
            )
        );
        assert_eq!(
            BILLING_SECTIONS,
            frontend_sections(
                include_str!("../../../frontend/src/lib/billing-sections.ts"),
                "BILLING_SECTIONS"
            )
        );
    }
    #[test]
    fn detail_links_encode_sections_without_changing_legacy_tabs() {
        for section in AGENT_DETAILS_SECTIONS {
            let path = AssistantPage::AgentDetails {
                agent: "a&b",
                tab: section,
            }
            .path();
            let url = url::Url::parse(&format!("https://nyxid.test{path}")).unwrap();
            assert!(
                url.query_pairs()
                    .any(|(key, value)| key == "details" && value == *section)
            );
        }
        for section in ACCOUNT_SETTINGS_SECTIONS {
            let path = AssistantPage::Settings { tab: section }.path();
            let url = url::Url::parse(&format!("https://nyxid.test{path}")).unwrap();
            assert!(
                url.query_pairs()
                    .any(|(key, value)| key == "panelTab" && value == *section)
            );
        }
        for section in BILLING_SECTIONS {
            let path = AssistantPage::Billing {
                tab: section.split('/').next().unwrap(),
                section: Some(section),
            }
            .path();
            let url = url::Url::parse(&format!("https://nyxid.test{path}")).unwrap();
            assert!(
                url.query_pairs()
                    .any(|(key, value)| key == "panelSection" && value == *section)
            );
        }
    }
}
