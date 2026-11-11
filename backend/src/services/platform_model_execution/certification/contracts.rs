//! Closed v2 accounting definitions, shared by certification and authoring discovery.
use crate::models::{
    platform_model::{DirectProvider, RouteTarget, VertexPublisher},
    platform_model_tariff::{CacheTtl, Modality, UsageContract, UsageFamily},
};
use serde::Serialize;

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FlagRule {
    Required,
    Optional,
    Forbidden,
}
impl FlagRule {
    fn accepts(self, value: bool) -> bool {
        match self {
            Self::Required => value,
            Self::Optional => true,
            Self::Forbidden => !value,
        }
    }
}
#[derive(Clone, Copy, Debug, Serialize)]
pub(crate) struct Flags {
    pub tokens: FlagRule,
    pub cache_read: FlagRule,
    pub split_reasoning: FlagRule,
    pub bill_tool_input: FlagRule,
    pub images: FlagRule,
    pub requests: FlagRule,
}
impl Flags {
    fn accepts(self, c: &UsageContract) -> bool {
        [
            (self.tokens, c.tokens),
            (self.cache_read, c.cache_read),
            (self.split_reasoning, c.split_reasoning),
            (self.bill_tool_input, c.bill_tool_input),
            (self.images, c.images),
            (self.requests, c.requests),
        ]
        .into_iter()
        .all(|(rule, value)| rule.accepts(value))
    }
}
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub(crate) enum Transport {
    #[serde(rename = "vertex.generateContent")]
    VertexGemini,
    #[serde(rename = "direct.messages")]
    DirectClaude,
    #[serde(rename = "vertex.rawPredict")]
    VertexClaude,
    #[serde(rename = "direct.responses")]
    DirectResponses,
}
impl Transport {
    fn for_route(route: &RouteTarget) -> Option<Self> {
        match route {
            RouteTarget::VertexPublisher {
                publisher: VertexPublisher::Google,
                ..
            } => Some(Self::VertexGemini),
            RouteTarget::VertexPublisher {
                publisher: VertexPublisher::Anthropic,
                ..
            } => Some(Self::VertexClaude),
            RouteTarget::DirectProvider {
                provider: DirectProvider::Anthropic,
                ..
            } => Some(Self::DirectClaude),
            RouteTarget::DirectProvider {
                provider: DirectProvider::Openai,
                ..
            } => Some(Self::DirectResponses),
            RouteTarget::VertexOpenapiMaas { .. } => None,
        }
    }
}
pub(crate) struct Definition {
    pub version: &'static str,
    pub family: UsageFamily,
    pub transports: &'static [Transport],
    pub fixture: &'static str,
    pub flags: Flags,
    pub cache_write_ttls: &'static [CacheTtl],
    pub input_modalities: &'static [Modality],
    pub output_modalities: &'static [Modality],
}
impl Definition {
    pub fn digest(&self) -> String {
        super::digest(self.fixture)
    }
    pub fn accepts(&self, c: &UsageContract) -> bool {
        c.version == self.version
            && c.family == self.family
            && c.fixture_sha256 == self.digest()
            && self.flags.accepts(c)
            && (c.input_modalities.is_empty() || c.input_modalities == self.input_modalities)
            && (c.output_modalities.is_empty() || c.output_modalities == self.output_modalities)
            && c.cache_write_ttls
                .iter()
                .all(|ttl| self.cache_write_ttls.contains(ttl))
    }
    pub fn accepts_route(&self, route: &RouteTarget) -> bool {
        Transport::for_route(route).is_some_and(|transport| self.transports.contains(&transport))
    }
    /// Complete aggregate accounting defaults; prices and bounds are never inferred.
    pub fn default_contract(&self) -> UsageContract {
        UsageContract {
            version: self.version.into(),
            fixture_sha256: self.digest(),
            family: self.family,
            tokens: true,
            cache_read: true,
            cache_write_ttls: self.cache_write_ttls.to_vec(),
            split_reasoning: false,
            input_modalities: vec![],
            output_modalities: vec![],
            bill_tool_input: self.flags.bill_tool_input != FlagRule::Forbidden,
            images: false,
            requests: false,
        }
    }
}
const FLAGS: Flags = Flags {
    tokens: FlagRule::Required,
    cache_read: FlagRule::Required,
    split_reasoning: FlagRule::Optional,
    bill_tool_input: FlagRule::Forbidden,
    images: FlagRule::Forbidden,
    requests: FlagRule::Optional,
};
pub(crate) const DEFINITIONS: &[Definition] = &[
    Definition {
        version: "gemini.generate.v2",
        family: UsageFamily::Gemini,
        transports: &[Transport::VertexGemini],
        fixture: include_str!("../fixtures/gemini-contract-v2.json"),
        flags: Flags {
            bill_tool_input: FlagRule::Optional,
            ..FLAGS
        },
        cache_write_ttls: &[],
        input_modalities: &[Modality::Image],
        output_modalities: &[],
    },
    Definition {
        version: "anthropic.messages.v2",
        family: UsageFamily::Claude,
        transports: &[Transport::DirectClaude, Transport::VertexClaude],
        fixture: include_str!("../fixtures/claude-contract-v2.json"),
        flags: Flags {
            split_reasoning: FlagRule::Forbidden,
            ..FLAGS
        },
        cache_write_ttls: &[CacheTtl::FiveMinutes, CacheTtl::OneHour],
        input_modalities: &[],
        output_modalities: &[],
    },
    Definition {
        version: "openai.responses.v2",
        family: UsageFamily::Openai,
        transports: &[Transport::DirectResponses],
        fixture: include_str!("../fixtures/responses-contract-v2.json"),
        flags: FLAGS,
        cache_write_ttls: &[CacheTtl::ThirtyMinutes, CacheTtl::TwelveHours],
        input_modalities: &[],
        output_modalities: &[],
    },
];
pub(crate) fn for_family(family: UsageFamily) -> Option<&'static Definition> {
    DEFINITIONS
        .iter()
        .find(|definition| definition.family == family)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn published_options_and_validator_have_identical_closed_rules() {
        for d in DEFINITIONS {
            let base = d.default_contract();
            assert!(d.accepts(&base));
            for (field, rule) in [
                ("tokens", d.flags.tokens),
                ("cache_read", d.flags.cache_read),
                ("split_reasoning", d.flags.split_reasoning),
                ("bill_tool_input", d.flags.bill_tool_input),
                ("images", d.flags.images),
                ("requests", d.flags.requests),
            ] {
                for value in [false, true] {
                    let mut json = serde_json::to_value(&base).unwrap();
                    json[field] = value.into();
                    assert_eq!(
                        d.accepts(&serde_json::from_value(json).unwrap()),
                        rule.accepts(value),
                        "{} {field}",
                        d.version
                    );
                }
            }
            for ttl in [
                CacheTtl::FiveMinutes,
                CacheTtl::OneHour,
                CacheTtl::ThirtyMinutes,
                CacheTtl::TwelveHours,
            ] {
                let mut c = base.clone();
                c.cache_write_ttls = vec![ttl];
                assert_eq!(d.accepts(&c), d.cache_write_ttls.contains(&ttl));
            }
            let mut c = base.clone();
            c.fixture_sha256 = "0".repeat(64);
            assert!(!d.accepts(&c));
            c = base.clone();
            c.version = "unknown.v2".into();
            assert!(!d.accepts(&c));
            c = base.clone();
            c.input_modalities = vec![Modality::Audio];
            assert!(!d.accepts(&c));
            c = base;
            c.output_modalities = vec![Modality::Image];
            assert!(!d.accepts(&c));
        }
    }
}
