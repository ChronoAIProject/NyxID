//! Browser destinations shared by assistant tools, setup APIs and notifications.
#[derive(Clone, Copy)]
pub enum AssistantPage<'a> {
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
        display: nyxid_machine::desktop::Display,
    },
}

impl AssistantPage<'_> {
    pub fn path(self) -> String {
        let (path, query) = match self {
            Self::Automations { setup } => {
                ("/assistant/automations".into(), setup.map(|v| ("setup", v)))
            }
            Self::Machines => ("/assistant/machines".into(), None),
            Self::MachineSettings { node } => {
                ("/assistant/machines".into(), Some(("machine", node)))
            }
            Self::SavedLogins => ("/assistant/machines".into(), Some(("tab", "logins"))),
            Self::MachineSetup { setup } => {
                ("/assistant/machines/new".into(), Some(("setup", setup)))
            }
            Self::MachinePair { code } => ("/assistant/machines/pair".into(), Some(("code", code))),
            Self::MachineDesktop {
                node,
                conversation,
                display,
            } => {
                let path = format!(
                    "/assistant/machines/{}/desktop",
                    url::form_urlencoded::byte_serialize(node.as_bytes()).collect::<String>()
                );
                let mut query = url::form_urlencoded::Serializer::new(String::new());
                if let Some(conversation) = conversation {
                    query.append_pair("conversation_id", conversation);
                }
                if display == nyxid_machine::desktop::Display::Dev {
                    query.append_pair("display", "dev");
                }
                let query = query.finish();
                return if query.is_empty() {
                    path
                } else {
                    format!("{path}?{query}")
                };
            }
        };
        match query {
            Some((key, value)) => format!(
                "{path}?{}",
                url::form_urlencoded::Serializer::new(String::new())
                    .append_pair(key, value)
                    .finish()
            ),
            None => path,
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
                    display: nyxid_machine::desktop::Display::Dev,
                },
                "/assistant/machines/n/desktop?display=dev",
            ),
            (
                MachineDesktop {
                    node: "n",
                    display: nyxid_machine::desktop::Display::Secure,
                    conversation: Some("nyxagent:c"),
                },
                "/assistant/machines/n/desktop?conversation_id=nyxagent%3Ac",
            ),
            (
                MachineDesktop {
                    node: "n",
                    display: nyxid_machine::desktop::Display::Secure,
                    conversation: None,
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
