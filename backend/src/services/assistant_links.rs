//! Browser destinations shared by assistant tools, setup APIs and notifications.
#[derive(Clone, Copy)]
pub enum AssistantPage<'a> {
    Settings {
        tab: &'a str,
    },
    NyxBotSettings,
    Billing {
        tab: &'a str,
    },
    Automations {
        setup: Option<&'a str>,
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
            Self::Settings { tab } => {
                query
                    .append_pair("panel", "settings")
                    .append_pair("panelTab", tab);
                "/assistant".into()
            }
            Self::NyxBotSettings => {
                query.append_pair("panel", "nyxbot");
                "/assistant".into()
            }
            Self::Billing { tab } => {
                query
                    .append_pair("panel", "billing")
                    .append_pair("panelTab", tab);
                "/assistant".into()
            }
            Self::Automations { setup } => {
                if let Some(setup) = setup {
                    query.append_pair("setup", setup);
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
            (NyxBotSettings, "/assistant?panel=nyxbot"),
            (
                Billing { tab: "billing" },
                "/assistant?panel=billing&panelTab=billing",
            ),
            (
                Billing { tab: "usage" },
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
                },
                "/assistant?panel=billing&panelTab=a%26b+c%2F%E4%B8%AD%E6%96%87%2B%25",
            ),
            (Automations { setup: None }, "/assistant/automations"),
            (
                Automations { setup: Some("a&b") },
                "/assistant/automations?setup=a%26b",
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
