//! Server-recorded child work. A task reference is never enough to admit a job
//! or a watch: its exact running row/lease, original actor and key are fenced too.
use super::*;
use crate::models::{
    async_service_operation::AsyncServiceOperation,
    machine_job::{MachineJob, MachineJobBinding},
};

pub(crate) enum Descendant {
    Job(Box<MachineJob>),
    Watch(Box<AsyncServiceOperation>),
}
type Identity<'a> = (
    &'a str,
    &'a str,
    &'a str,
    &'a str,
    Option<&'a str>,
    Option<&'a TaskAdmission>,
);

impl Descendant {
    fn identity(&self) -> Identity<'_> {
        match self {
            Self::Job(j) => (
                &j.user_id,
                &j.api_key_id,
                &j.agent_id,
                &j.conversation_id,
                j.issuing_turn_id.as_deref(),
                j.execution_task.as_ref(),
            ),
            Self::Watch(w) => (
                &w.user_id,
                &w.api_key_id,
                &w.agent_id,
                &w.conversation_id,
                Some(&w.turn_id),
                w.execution_task.as_ref(),
            ),
        }
    }
    fn selector(&self) -> AppResult<(&'static str, Document)> {
        let (actor, key, agent, conversation, turn, task) = self.identity();
        let task = task.ok_or_else(refused)?;
        if task.actor_user_id != actor || task.parent_turn_id.as_deref() != turn {
            return Err(refused());
        }
        let mut filter = doc! {"user_id":actor,"api_key_id":key,"agent_id":agent,"conversation_id":conversation,
        "execution_task":bson::to_bson(task).map_err(encoding)?,"expires_at":{"$gt":bson::DateTime::now()}};
        Ok(match self {
            Self::Job(j) => {
                if !matches!(&j.binding, Some(MachineJobBinding::Assistant {version:1,conversation_id,turn_id,api_key_id,agent_id}) if conversation_id == conversation && Some(turn_id.as_str()) == turn && api_key_id == key && agent_id == agent)
                {
                    return Err(refused());
                }
                filter.extend(doc! {"_id":&j.id,"state":"running","issuing_turn_id":turn,"binding":bson::to_bson(&j.binding).map_err(encoding)?,"node_id":&j.node_id,"runtime_id":&j.runtime_id});
                ("machine_jobs", filter)
            }
            Self::Watch(w) => {
                if w.lease_id.is_none()
                    || super::super::cma_operation::poll_id(w).is_none()
                    || !matches!(w.state.as_str(), "waiting" | "cancelling")
                {
                    return Err(refused());
                }
                filter.extend(doc! {"_id":&w.id,"turn_id":turn,"state":&w.state,"lease_id":&w.lease_id,"lease_until":{"$gt":bson::DateTime::now()},"service_id":&w.service_id,"operation_id":&w.operation_id,"contract_binding":&w.contract_binding});
                ("assistant_async_operations", filter)
            }
        })
    }
    pub(super) async fn check(
        &self,
        db: &Database,
        session: Option<&mut ClientSession>,
    ) -> AppResult<()> {
        let (collection, filter) = self.selector()?;
        let task = self.identity().5.ok_or_else(refused)?;
        let rows = db.collection::<Document>(collection);
        let matched = if let Some(session) = session {
            tasks::fence(db, session, task).await?;
            rows.update_one(filter, doc! {"$inc":{"execution_dispatch_fence":1_i64}})
                .session(session)
                .await?
                .matched_count
                == 1
        } else {
            tasks::check(db, task).await?;
            rows.find_one(filter)
                .projection(doc! {"_id":1})
                .await?
                .is_some()
        };
        if !matched {
            return Err(refused());
        }
        Ok(())
    }
}

/// The caller has already verified the job signature or watch lease. Restore
/// only its persisted lineage, never the conversation's newer requester's turn.
pub(crate) fn attach(auth: &mut crate::mw::auth::AuthUser, source: Descendant) -> AppResult<()> {
    let (actor, key, agent, conversation, turn, task) = source.identity();
    let Some(task) = task else {
        super::exclude(auth);
        return Ok(());
    };
    let current = auth.execution_permission.as_ref();
    let mut chat = auth.assistant_chat.as_deref().ok_or_else(refused)?.clone();
    if auth.user_id.to_string() != actor
        || auth.api_key_id.as_deref() != Some(key)
        || chat.agent_id != agent
        || chat.conversation_id != conversation
        || chat.guest
        || chat.execution_unsupported
    {
        return Err(refused());
    }
    chat.turn_id = turn.map(str::to_owned);
    chat.execution_task = Some(task.clone());
    chat.invocation_controlled = task.task.root_kind == TaskRootKind::Invocation;
    chat.invocation_id = chat
        .invocation_controlled
        .then(|| task.task.root_id.clone());
    let mut context = Arc::try_unwrap(
        RequestContext::enrolled(Some(VERSION), actor, key, Some(&chat)).ok_or_else(refused)?,
    )
    .map_err(|_| refused())?;
    if let Some(expected) = current.and_then(|c| c.authenticated_key.get()) {
        let _ = context.authenticated_key.set(expected.clone());
    }
    if let Some(current) = current {
        context.operation_id.clone_from(&current.operation_id);
        context.resume = current.resume().cloned();
        if let Some(v) = current.ingress.get() {
            let _ = context.ingress.set(v.clone());
        }
    }
    context.descendant = Some(Arc::new(source));
    let context = Arc::new(context);
    chat.execution_permission = Some(context.clone());
    auth.execution_permission = Some(context);
    auth.assistant_chat = Some(Arc::new(chat));
    Ok(())
}
