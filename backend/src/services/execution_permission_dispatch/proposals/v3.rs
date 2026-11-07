//! Explicit private-conversation choice. B2 retains the v2 source/adapter envelope.
//! Assistant v1 and v3 share the same effect state machine and retained payload format.
use super::*;

pub(crate) const READINESS: &str = "permissions:execution-authoring-private-conversations-v1";
pub(crate) const CHOICE: &str = "persistent_private_conversations";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AssistantVersion {
    V1,
    V3,
}

impl AssistantVersion {
    pub(crate) const ALL: [Self; 2] = [Self::V1, Self::V3];

    pub(crate) const fn number(self) -> u32 {
        match self {
            Self::V1 => 1,
            Self::V3 => 3,
        }
    }

    pub(crate) fn of(row: &ApprovalRequest) -> AppResult<Self> {
        let e = row.execution_proposal.as_ref().ok_or_else(conflict)?;
        match e.version {
            1 if e.v1.is_some() && e.v3.is_none() => Ok(Self::V1),
            3 if e.v3.is_some() && e.v1.is_none() && e.v2.is_none() => Ok(Self::V3),
            _ => Err(conflict()),
        }
    }

    pub(super) fn envelope(self, p: Proposal) -> ExecutionProposal {
        let (v1, v3) = match self {
            Self::V1 => (Some(p), None),
            Self::V3 => (None, Some(p)),
        };
        ExecutionProposal {
            version: self.number(),
            v1,
            v2: None,
            v3,
        }
    }

    pub(crate) fn selector(self, template: Document) -> Document {
        let mut result = self.update(template);
        result.insert("execution_proposal.version", self.number() as i32);
        result
    }

    pub(crate) fn update(self, template: Document) -> Document {
        rebase(template, self.number())
    }

    pub(super) async fn require(self, db: &Database) -> AppResult<()> {
        if self == Self::V3 {
            crate::services::security_readiness_service::require(db, READINESS).await?;
        }
        Ok(())
    }
}

/// Rebase only static, server-authored query/update templates, never effect data.
/// v2 uses this solely to retain the existing shared cancellation selector.
pub(super) fn rebase(template: Document, version: u32) -> Document {
    fn value(v: bson::Bson, version: u32) -> bson::Bson {
        match v {
            bson::Bson::Document(d) => bson::Bson::Document(rebase(d, version)),
            bson::Bson::Array(items) => {
                bson::Bson::Array(items.into_iter().map(|v| value(v, version)).collect())
            }
            v => v,
        }
    }
    template
        .into_iter()
        .map(|(key, v)| {
            let key = key.strip_prefix("execution_proposal.v1.").map_or_else(
                || key.clone(),
                |field| format!("execution_proposal.v{version}.{field}"),
            );
            (key, value(v, version))
        })
        .collect()
}

pub(crate) fn assistant_selector(template: Document) -> Document {
    doc! {"$or": AssistantVersion::ALL.map(|v| v.selector(template.clone())).to_vec()}
}

pub(crate) fn source_selector(template: Document) -> Document {
    let mut selectors = AssistantVersion::ALL
        .map(|v| v.selector(template.clone()))
        .to_vec();
    let mut second = rebase(template, 2);
    second.insert("execution_proposal.version", 2);
    selectors.push(second);
    doc! {"$or":selectors}
}

fn eligible(c: &ExecutionContract) -> bool {
    c.subject.lane == ExecutionLane::Owner
        && matches!(c.subject.principal, Principal::Agent(_))
        && c.binding.owner_user_id == c.subject.actor_user_id
        && matches!(&c.context.audience, Audience::Private { user_id } if user_id == &c.subject.actor_user_id)
        && matches!(c.context.conversation, ConversationScope::Exact { .. })
}

pub(super) async fn choose_version(
    db: &Database,
    c: &ExecutionContract,
    reusable: bool,
) -> AppResult<AssistantVersion> {
    if reusable
        && eligible(c)
        && crate::services::security_readiness_service::report(db, READINESS)
            .await?
            .ready
    {
        Ok(AssistantVersion::V3)
    } else {
        Ok(AssistantVersion::V1)
    }
}

fn choice_digest(contract: &ExecutionContract) -> AppResult<String> {
    crate::services::audit_service::keyed_fingerprint(
        Some(audit_key()?),
        b"execution-proposal-private-conversations-v3",
        &serde_json::to_vec(&json!({"choice":CHOICE,"contract":contract})).map_err(encoding)?,
    )
    .ok_or_else(refused)
}

pub(super) fn choice(contract: &ExecutionContract) -> AppResult<Choice> {
    if !eligible(contract) {
        return Err(refused());
    }
    let mut selected = contract.clone();
    selected.context.conversation = ConversationScope::SameAudience;
    selected.validity = Validity::Persistent;
    Ok(Choice {
        id: CHOICE.into(),
        digest: choice_digest(&selected)?,
        validity: Validity::Persistent,
        label: "This agent, this connection and the displayed operation/inputs, across my private conversations, until revoked".into(),
        conversation_scope: Some(ConversationScope::SameAudience),
    })
}

pub(super) fn selected_contract(
    version: AssistantVersion,
    p: &Proposal,
    choice: &Choice,
) -> AppResult<ExecutionContract> {
    let mut selected = p.contract.clone();
    selected.validity = choice.validity.clone();
    if choice.conversation_scope.is_some() || choice.id == CHOICE {
        if version != AssistantVersion::V3
            || choice.id != CHOICE
            || choice.conversation_scope != Some(ConversationScope::SameAudience)
            || !matches!(choice.validity, Validity::Persistent)
            || !eligible(&p.contract)
        {
            return Err(conflict());
        }
        selected.context.conversation = ConversationScope::SameAudience;
        if choice.digest != choice_digest(&selected)? {
            return Err(conflict());
        }
    }
    Ok(selected)
}

/// Human display only. Never used to decide authority or compile a contract.
pub(crate) fn context_scope(context: &ExecutionContext) -> &'static str {
    match (&context.conversation, &context.audience) {
        (ConversationScope::Exact { .. }, _) => "current_conversation",
        (ConversationScope::SameAudience, Audience::Private { .. }) => "private_conversations",
        (ConversationScope::SameAudience, _) => "same_audience",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn personal() -> ExecutionContract {
        let mut c = contract::tests::fixture().execution.unwrap().v1.unwrap();
        c.subject.principal = Principal::Agent("agent".into());
        c.subject.lane = ExecutionLane::Owner;
        c.binding.owner_user_id = c.subject.actor_user_id.clone();
        c.context = ExecutionContext {
            audience: Audience::Private {
                user_id: c.subject.actor_user_id.clone(),
            },
            conversation: ConversationScope::Exact {
                conversation_id: "thread".into(),
            },
        };
        c
    }

    #[test]
    fn private_conversations_choice_is_personal_owner_and_private_only() {
        let c = personal();
        assert!(eligible(&c));
        for lane in [
            ExecutionLane::Shared,
            ExecutionLane::OrgMember,
            ExecutionLane::Guest,
        ] {
            let mut other = c.clone();
            other.subject.lane = lane;
            assert!(!eligible(&other));
        }
        for audience in [
            Audience::Private {
                user_id: "someone-else".into(),
            },
            Audience::Group {
                group_id: "group".into(),
                revision: 1,
            },
            Audience::Channel {
                binding_id: "channel".into(),
                revision: 1,
            },
        ] {
            let mut other = c.clone();
            other.context.audience = audience;
            assert!(!eligible(&other));
        }
        let mut other = c.clone();
        other.binding.owner_user_id = "org-or-another-person".into();
        assert!(!eligible(&other));
        other = c;
        other.subject.principal = Principal::ApiKey("external".into());
        assert!(!eligible(&other));
    }

    #[test]
    fn private_conversations_selectors_bind_versions_and_preserve_literal_values() {
        let template = doc! {"user_id":"owner","$or":[
            {"execution_proposal.v1.state":"queued"},
            {"execution_proposal.v1.failure_code":"execution_proposal.v1.literal"}
        ]};
        let translated = AssistantVersion::V3.selector(template.clone());
        assert_eq!(translated.get_i32("execution_proposal.version").unwrap(), 3);
        assert_eq!(
            translated.get_array("$or").unwrap()[1]
                .as_document()
                .unwrap()
                .get_str("execution_proposal.v3.failure_code")
                .unwrap(),
            "execution_proposal.v1.literal"
        );
        let both = assistant_selector(template.clone());
        assert_eq!(both.get_array("$or").unwrap().len(), 2);
        assert_eq!(source_selector(template).get_array("$or").unwrap().len(), 3);
    }
}
