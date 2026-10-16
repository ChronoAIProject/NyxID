//! External Machine execution over the existing grant, context, lease and job
//! stores. Authentication enters only as `VerifiedExternalSource`; request
//! arguments never carry an execution identity.
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        machine_access as access,
        machine_external_execution::{
            self as execution, ConfirmationReviewParameter, ExternalConfirmation, ExternalContext,
            ExternalLease,
        },
        machine_external_grant::{
            ExternalGrantBinding, ExternalPrincipalRef, ExternalRevocationSelector,
        },
        machine_job::{
            self, DeclaredService, MachineCancellationRequester, MachineJob, MachineJobBinding,
            MachineSystemCancellationReason,
        },
        node::{self, Node},
    },
    services::{
        api_key_mutation_service as transactions,
        machine_external_grant_service::{self as grants, AdmittedGrant, VerifiedExternalSource},
    },
};
use chrono::{DateTime, Duration, Utc};
use futures::TryStreamExt;
use mongodb::{
    ClientSession, Database, IndexModel,
    bson::{self, Bson, Document, doc},
    options::{IndexOptions, ReturnDocument},
};
use nyxid_machine::{
    Operation,
    authority::{ExternalAuthority, ExternalPrincipal, OAuthConsent},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

const HISTORY_DAYS: i64 = 30;
const MAX_JOBS_PAGE: i64 = 50;

pub fn principal_user(principal: &ExternalPrincipalRef) -> &str {
    match principal {
        ExternalPrincipalRef::OAuth { user_id, .. }
        | ExternalPrincipalRef::ApiKey { user_id, .. } => user_id,
    }
}
pub fn same_principal(a: &ExternalPrincipalRef, b: &ExternalPrincipalRef) -> bool {
    match (a, b) {
        (
            ExternalPrincipalRef::OAuth {
                user_id: a,
                oauth_client_id: ac,
            },
            ExternalPrincipalRef::OAuth {
                user_id: b,
                oauth_client_id: bc,
            },
        ) => a == b && ac == bc,
        (
            ExternalPrincipalRef::ApiKey {
                user_id: a,
                api_key_id: ak,
                ..
            },
            ExternalPrincipalRef::ApiKey {
                user_id: b,
                api_key_id: bk,
                ..
            },
        ) => a == b && ak == bk,
        _ => false,
    }
}
fn wire_principal(principal: &ExternalPrincipalRef) -> ExternalPrincipal {
    match principal {
        ExternalPrincipalRef::OAuth {
            user_id,
            oauth_client_id,
        } => ExternalPrincipal::OAuth {
            user_id: user_id.clone(),
            oauth_client_id: oauth_client_id.clone(),
        },
        ExternalPrincipalRef::ApiKey {
            user_id,
            api_key_id,
            api_key_credential_id,
        } => ExternalPrincipal::ApiKey {
            user_id: user_id.clone(),
            api_key_id: api_key_id.clone(),
            api_key_credential_id: api_key_credential_id.clone(),
        },
    }
}
fn encode<T: serde::Serialize>(value: &T) -> AppResult<Bson> {
    bson::to_bson(value).map_err(|_| AppError::Internal("Machine execution encoding failed".into()))
}
fn refused() -> AppError {
    AppError::MachinePermissionRevoked
}
fn invalid(message: &str) -> AppError {
    AppError::ValidationError(message.into())
}

pub async fn ensure_indexes(db: &Database) -> AppResult<()> {
    for (collection, keys) in [
        (
            machine_job::COLLECTION_NAME,
            doc! {"user_id":1,"binding.grant_id":1,"node_id":1,"created_at":-1,"_id":-1},
        ),
        (
            machine_job::COLLECTION_NAME,
            doc! {"state":1,"expires_at":1},
        ),
        (
            machine_job::COLLECTION_NAME,
            doc! {"state":1,"retain_until":1,"finished_at":1},
        ),
        (
            access::LEASES,
            doc! {"kind":1,"external_authority.expires_at_ms":1},
        ),
        (
            access::LEASES,
            doc! {"external_authority.grant_id":1,"external_authority.grant_revision":1},
        ),
        (
            access::LEASES,
            doc! {"external_authority.principal.api_key_credential_id":1},
        ),
        (
            execution::CONFIRMATIONS,
            doc! {"user_id":1,"created_at":-1,"_id":-1},
        ),
        (
            execution::CONFIRMATIONS,
            doc! {"binding_digest":1,"state":1,"expires_at":1},
        ),
    ] {
        db.collection::<Document>(collection)
            .create_index(IndexModel::builder().keys(keys).build())
            .await?;
    }
    db.collection::<Document>(access::CONTEXTS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"node_id":1,"grant_id":1})
                .options(
                    IndexOptions::builder()
                        .name(Some("external_context".to_string()))
                        .unique(true)
                        .partial_filter_expression(doc! {"kind":"external"})
                        .build(),
                )
                .build(),
        )
        .await?;
    // Only terminal, explicitly projected rows have this field. Execution
    // expiry alone never purges job history or in-flight cleanup evidence.
    db.collection::<Document>(machine_job::COLLECTION_NAME)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"retain_until":1})
                .options(
                    IndexOptions::builder()
                        .expire_after(std::time::Duration::ZERO)
                        .build(),
                )
                .build(),
        )
        .await?;
    Ok(())
}

async fn active_grant(
    db: &Database,
    source: &VerifiedExternalSource,
) -> AppResult<crate::models::machine_external_grant::MachineExternalGrant> {
    let result = match source.principal() {
        ExternalPrincipalRef::OAuth {
            user_id,
            oauth_client_id,
        } => grants::get_active_for_oauth(db, &user_id, &oauth_client_id).await?,
        ExternalPrincipalRef::ApiKey {
            user_id,
            api_key_id,
            ..
        } => grants::get_active_for_key(db, &user_id, &api_key_id).await?,
    };
    result.ok_or_else(refused)
}

/// Discovery returns only live-authorized shell/files machines; each authority
/// read is fenced in the same transaction used by source mutations.
pub async fn visible(db: &Database, source: &VerifiedExternalSource) -> AppResult<Vec<Value>> {
    let grant = match active_grant(db, source).await {
        Ok(g) => g,
        Err(AppError::MachinePermissionRevoked) => return Ok(vec![]),
        Err(e) => return Err(e),
    };
    let ids: Vec<_> = grant.policy.assignments.keys().take(500).cloned().collect();
    let rows: Vec<Node> = db
        .collection(node::COLLECTION_NAME)
        .find(doc! {"_id":{"$in":ids},"user_id":&grant.user_id,"is_active":true})
        .sort(doc! {"name":1,"_id":1})
        .limit(500)
        .await?
        .try_collect()
        .await?;
    let mut result = Vec::new();
    for node in rows {
        if node.machine.as_ref().is_none_or(|p| !p.authority_v3()) {
            continue;
        }
        let admitted = admit_read(db, source, &node.id, &grant.id, grant.revision).await;
        match admitted {
            Ok(a) => {
                result.push(json!({"id":node.id,"name":node.name,"status":node.status,"capabilities":a.assignment.capabilities,"file_roots":a.restrictions.file_roots,"services":a.restrictions.service_ids,"machine_confirm":node.machine_confirm,"grant_id":a.grant.id,"grant_revision":a.grant.revision,"grant_expires_at":a.grant.expires_at,"mode":"shared_legacy","execution_note":super::machine_tools::execution_note("shared_legacy",false)}));
            }
            Err(
                AppError::MachineNotAllowed
                | AppError::MachinePermissionRevoked
                | AppError::NodeOffline(_),
            ) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(result)
}

async fn resolve_node(
    db: &Database,
    source: &VerifiedExternalSource,
    reference: &str,
) -> AppResult<Node> {
    if reference.is_empty() || reference.len() > 200 {
        return Err(invalid("Machine name or ID is required"));
    }
    let principal = source.principal();
    let rows:Vec<Node>=db.collection(node::COLLECTION_NAME).find(doc!{"user_id":principal_user(&principal),"$or":[{"_id":reference},{"name":reference}]}).limit(2).await?.try_collect().await?;
    if rows.len() != 1 {
        return Err(AppError::MachineNotAllowed);
    }
    Ok(rows.into_iter().next().expect("one node"))
}

type ExternalJobBinding<'a> = (
    &'a ExternalPrincipalRef,
    &'a str,
    i64,
    i64,
    DateTime<Utc>,
    &'a [String],
    &'a str,
);

fn external_binding(job: &MachineJob) -> AppResult<ExternalJobBinding<'_>> {
    match job.binding.as_ref() {
        Some(MachineJobBinding::External {
            principal,
            grant_id,
            grant_revision,
            assignment_revision,
            authentication_expires_at,
            service_ceiling,
            lease_id,
            ..
        }) => Ok((
            principal,
            grant_id,
            *grant_revision,
            *assignment_revision,
            *authentication_expires_at,
            service_ceiling,
            lease_id,
        )),
        _ => Err(AppError::MachineJobNotFound),
    }
}

async fn source_for_job(_db: &Database, job: &MachineJob) -> AppResult<VerifiedExternalSource> {
    // Persistent, exact issue-time authority works after restart and on every
    // replica. This creates no credential and cannot widen the original cutoff.
    VerifiedExternalSource::from_persisted_job(job)
}

#[derive(Clone)]
struct Admission {
    authority: Box<ExternalAuthority>,
    job: Option<MachineJob>,
    pending: Option<ExternalConfirmation>,
    replayed: bool,
}

fn confirmation_digest(
    source: &VerifiedExternalSource,
    node: &Node,
    a: &AdmittedGrant,
    runtime_generation: Option<u64>,
    epoch: Option<u64>,
    operation: Operation,
    args: &Value,
) -> String {
    super::assistant_acknowledgement_service::arguments_digest(
        &json!({"principal":source.principal(),"protected_consent":source.protected_consent(),"grant_id":a.grant.id,"grant_revision":a.grant.revision,"assignment_revision":a.assignment.revision,"node_id":node.id,"runtime_id":node.machine.as_ref().map(|p|&p.runtime_id),"runtime_generation":runtime_generation,"credential_epoch":epoch,"operation":operation,"parameters_digest":super::assistant_acknowledgement_service::arguments_digest(args)}),
    )
}

fn confirmation_review(
    operation: Operation,
    args: &Value,
    services: &[DeclaredService],
) -> Vec<ConfirmationReviewParameter> {
    let mut review = vec![ConfirmationReviewParameter {
        name: "Operation".into(),
        value: format!("{operation:?}"),
    }];
    for (field, label) in [
        ("command", "Shell command"),
        ("stdin", "Standard input"),
        ("content", "File content"),
        ("old_text", "Existing edit text"),
        ("new_text", "Replacement edit text"),
    ] {
        if let Some(value) = args.get(field).and_then(Value::as_str) {
            review.push(ConfirmationReviewParameter {
                name: label.into(),
                value: format!(
                    "{} bytes; content is not retained or shown; SHA-256 {}",
                    value.len(),
                    hex::encode(Sha256::digest(value.as_bytes()))
                ),
            });
        }
    }
    if operation == Operation::Exec {
        review.push(ConfirmationReviewParameter {
            name: "Maximum command duration".into(),
            value: format!(
                "{} seconds",
                args.get("timeout_secs")
                    .and_then(Value::as_u64)
                    .unwrap_or(120)
            ),
        });
        review.push(ConfirmationReviewParameter { name: "Execution boundary".into(), value: "Shared shell account; command text may perform additional actions. This summary does not verify the command's effects.".into() });
    }
    if let Some(path) = args.get("path").and_then(Value::as_str) {
        review.push(ConfirmationReviewParameter {
            name: "Target path".into(),
            value: format!(
                "{} bytes; path is not retained or shown; SHA-256 {}",
                path.len(),
                hex::encode(Sha256::digest(path.as_bytes()))
            ),
        });
    }
    review.push(ConfirmationReviewParameter {
        name: "Declared services".into(),
        value: if services.is_empty() {
            "None".into()
        } else {
            services
                .iter()
                .map(|s| s.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        },
    });
    review
}

async fn admit_request(
    state: &AppState,
    source: &VerifiedExternalSource,
    node: &Node,
    operation: Operation,
    args: &Value,
    existing: Option<&MachineJob>,
) -> AppResult<Admission> {
    let profile = node
        .machine
        .as_ref()
        .filter(|p| p.authority_v3())
        .ok_or(AppError::MachineAuthorityUnsupported)?;
    super::machine_service::capable(node, operation)?;
    let timeout = args
        .get("timeout_secs")
        .map(Value::as_u64)
        .unwrap_or(Some(120))
        .filter(|v| (1..=3600).contains(v))
        .ok_or_else(|| invalid("Command timeout must be between 1 and 3600 seconds"))?;
    let expected = existing
        .map(|j| external_binding(j).map(|(_, id, revision, _, _, _, _)| (id.to_owned(), revision)))
        .transpose()?;
    let db = state.db.clone();
    let source = source.clone();
    let node = node.clone();
    let args = args.clone();
    let profile = profile.clone();
    let existing = existing.cloned();
    let job_id = existing
        .as_ref()
        .map(|j| j.id.clone())
        .or_else(|| (operation == Operation::Exec).then(|| Uuid::new_v4().to_string()));
    let lease_id = Uuid::new_v4().to_string();
    let context_id = Uuid::new_v4().to_string();
    let mut session = db.client().start_session().await?;
    let admitted=session.start_transaction().write_concern(mongodb::options::WriteConcern::majority()).and_run2(async move |session|{
        let result:AppResult<Admission>=async {
            let mut a=grants::admit_in_session(&db,&source,&node.id,expected.as_ref().map(|(id,r)|(id.as_str(),*r)),session).await?;
            let node=db.collection::<Node>(node::COLLECTION_NAME).find_one(doc!{"_id":&node.id,"user_id":&node.user_id,"is_active":true,"machine.runtime_id":&profile.runtime_id}).session(&mut *session).await?.ok_or(AppError::MachineAuthorityStale)?;
            super::machine_service::capable(&node,operation)?;
            if node.machine.as_ref().is_none_or(|p|!p.authority_v3()){return Err(AppError::MachineAuthorityUnsupported);}
            if !a.assignment.capabilities.allows(operation,&args){return Err(refused());}
            if let Some(job)=&existing {let (principal,_,_,revision,_,_,_)=external_binding(job)?;if !same_principal(principal,&source.principal())||revision!=a.assignment.revision{return Err(refused());}}
            let runtime=super::remote_machine_runtime::fence_execution_in_session(&db,&node,session).await?;
            let now=Utc::now();
            let deadline=now+Duration::seconds(if operation==Operation::Exec{timeout as i64}else{60});
            let cutoff=(now+Duration::milliseconds(nyxid_machine::authority::LEASE_MS)).min(deadline).min(a.authentication_expires_at).min(runtime.funding_cutoff.unwrap_or(DateTime::<Utc>::MAX_UTC));
            if cutoff<=now{return Err(refused());}
            let context=ExternalContext{id:context_id.clone(),kind:"external".into(),node_id:node.id.clone(),owner_id:node.user_id.clone(),grant_id:a.grant.id.clone(),generation:1,mode:"shared_legacy".into()};
            let context=db.collection::<ExternalContext>(access::CONTEXTS).find_one_and_update(doc!{"kind":"external","node_id":&node.id,"grant_id":&a.grant.id},doc!{"$setOnInsert":bson::to_document(&context).map_err(|_|refused())?}).upsert(true).return_document(ReturnDocument::After).session(&mut *session).await?.ok_or_else(refused)?;
            let requested_services=args.get("services").cloned().unwrap_or_else(||json!([]));
            let service_names:Vec<String>=serde_json::from_value(requested_services).map_err(|_|invalid("services must contain service IDs or slugs"))?;
            if service_names.len()>32{return Err(invalid("At most 32 services may be declared"));}
            let available=super::machine_gateway_service::external_services(&db,&node.user_id,&a.restrictions.service_ids).await?;
            let declared=if let Some(job)=&existing{job.services.iter().filter(|service|available.iter().any(|live|live.id==service.id&&live.slug==service.slug)).cloned().collect()}else{super::machine_gateway_service::declared(&super::machine_gateway_service::declare(&service_names,available)?)};
            let mut authority=Box::new(ExternalAuthority{
                principal:wire_principal(&source.principal()),oauth_consent:source.protected_consent().map(|c|OAuthConsent{consent_id:c.consent_id.clone(),consent_revision:c.consent_revision.clone()}),
                context_id:context.id,generation:1,mode:"shared_legacy".into(),owner_id:node.user_id.clone(),actor_id:principal_user(&source.principal()).to_owned(),node_id:node.id.clone(),runtime_id:profile.runtime_id.clone(),runtime_generation:runtime.runtime_generation,credential_epoch:runtime.credential_epoch,
                grant_id:a.grant.id.clone(),grant_revision:a.grant.revision,assignment_revision:a.assignment.revision,lease_id:lease_id.clone(),expires_at_ms:cutoff.timestamp_millis(),auth_expires_at_ms:a.authentication_expires_at.timestamp_millis(),job_expires_at_ms:deadline.timestamp_millis(),funding_expires_at_ms:runtime.funding_cutoff.map(|x|x.timestamp_millis()),capabilities:a.assignment.capabilities,file_roots:a.restrictions.file_roots.clone(),services:declared.iter().map(|s|nyxid_machine::authority::DeclaredService{id:s.id.clone(),slug:s.slug.clone()}).collect(),job_id:job_id.clone(),
            });
            if super::machine_service::confirmation(&node,operation,&args) && operation!=Operation::JobCancel {
                let digest=confirmation_digest(&source,&node,&a,runtime.runtime_generation,runtime.credential_epoch,operation,&args);
                let supplied=args.get("acknowledgement_id").and_then(Value::as_str);
                let mut filter=doc!{"user_id":&node.user_id,"binding_digest":&digest};if let Some(id)=supplied{filter.insert("_id",id);}else{filter.insert("state",doc!{"$in":["pending","approved"]});filter.insert("expires_at",doc!{"$gt":bson::DateTime::from_chrono(now)});}
                if let Some(c)=db.collection::<ExternalConfirmation>(execution::CONFIRMATIONS).find_one(filter).sort(doc!{"created_at":-1}).session(&mut *session).await? {
                    if c.state=="consumed" {
                        let replay=c.job_id.as_ref().map(|id|doc!{"_id":id,"binding.grant_id":&a.grant.id});
                        let job=if let Some(filter)=replay{db.collection::<MachineJob>(machine_job::COLLECTION_NAME).find_one(filter).session(&mut *session).await?}else{None};
                        return Ok(Admission{authority,job,pending:None,replayed:true});
                    }
                    if c.expires_at<=now{return Err(AppError::MachineConfirmationDeclined);}
                    let original_source=VerifiedExternalSource::from_persisted_confirmation(&c)?;
                    let original=grants::admit_in_session(&db,&original_source,&node.id,Some((&c.grant_id,c.grant_revision)),session).await?;
                    if c.principal!=source.principal()||c.protected_consent.as_ref()!=source.protected_consent()||original.assignment.revision!=c.assignment_revision{return Err(refused());}
                    a.authentication_expires_at=a.authentication_expires_at.min(original.authentication_expires_at);
                    a.restrictions.service_ids.retain(|id|original.restrictions.service_ids.contains(id));
                    authority.services.retain(|service|a.restrictions.service_ids.contains(&service.id));
                    authority.auth_expires_at_ms=a.authentication_expires_at.timestamp_millis();
                    authority.expires_at_ms=authority.expires_at_ms.min(authority.auth_expires_at_ms).min(c.expires_at.timestamp_millis());
                    if c.state=="declined"{return Err(AppError::MachineConfirmationDeclined);}
                    if c.state!="approved"{return Ok(Admission{authority,job:None,pending:Some(c),replayed:false});}
                    let consumed=db.collection::<Document>(execution::CONFIRMATIONS).update_one(doc!{"_id":&c.id,"state":"approved","expires_at":{"$gt":bson::DateTime::from_chrono(now)}},doc!{"$set":{"state":"consumed","consumed_at":bson::DateTime::from_chrono(now),"job_id":&job_id}}).session(&mut *session).await?;
                    if consumed.matched_count!=1{return Err(AppError::MachineConfirmationDeclined);}
                } else {
                    if supplied.is_some(){return Err(AppError::MachineConfirmationDeclined);}
                    let c=ExternalConfirmation{id:Uuid::new_v4().to_string(),user_id:node.user_id.clone(),node_id:node.id.clone(),principal:source.principal(),protected_consent:source.protected_consent().cloned(),authentication_expires_at:Some(a.authentication_expires_at),service_ceiling:a.restrictions.service_ids.clone(),grant_id:a.grant.id.clone(),grant_revision:a.grant.revision,assignment_revision:a.assignment.revision,runtime_id:profile.runtime_id.clone(),runtime_generation:runtime.runtime_generation,credential_epoch:runtime.credential_epoch,operation,parameters_digest:super::assistant_acknowledgement_service::arguments_digest(&args),safe_parameters:confirmation_review(operation,&args,&declared),binding_digest:digest,state:"pending".into(),job_id:None,created_at:now,expires_at:(now+Duration::seconds(super::assistant_acknowledgement_service::PENDING_SECONDS)).min(a.authentication_expires_at),decided_at:None,consumed_at:None};
                    db.collection::<ExternalConfirmation>(execution::CONFIRMATIONS).insert_one(&c).session(&mut *session).await?;
                    return Ok(Admission{authority,job:None,pending:Some(c),replayed:false});
                }
            }
            let fence=db.collection::<Document>(node::COLLECTION_NAME).update_one(doc!{"_id":&node.id,"is_active":true,"machine.runtime_id":&profile.runtime_id},doc!{"$inc":{"machine_authority_admission_fence":1_i64}}).session(&mut *session).await?;
            if fence.matched_count!=1{return Err(AppError::MachineAuthorityStale);}
            if db.collection::<Document>(access::LEASES).count_documents(doc!{"node_id":&node.id,"$or":[{"authority.expires_at_ms":{"$gt":now.timestamp_millis()}},{"external_authority.expires_at_ms":{"$gt":now.timestamp_millis()}}]}).session(&mut *session).await? >= super::machine_access_service::MAX_NODE_LEASES { return Err(AppError::MachineAuthorityBusy); }
            let lease=ExternalLease{id:lease_id.clone(),kind:"external".into(),node_id:node.id.clone(),job_id:if operation==Operation::Exec {job_id.clone()}else{None},external_authority:authority.clone(),expires_at:deadline+Duration::days(1)};
            db.collection::<ExternalLease>(access::LEASES).insert_one(&lease).session(&mut *session).await?;
            let job=if operation==Operation::Exec{
                let job=MachineJob{id:job_id.clone().expect("exec job"),user_id:node.user_id.clone(),node_id:node.id.clone(),runtime_id:profile.runtime_id.clone(),runtime_generation:runtime.runtime_generation,conversation_id:String::new(),issuing_turn_id:None,api_key_id:String::new(),agent_id:String::new(),state:"running".into(),exit_code:None,stop_reason:None,duration_ms:None,cancellation_requested_by:None,cancellation_requested_at:None,services:declared,report_to:None,report_provenance:0,created_at:now,expires_at:deadline,finished_at:None,retain_until:None,
                    binding:Some(MachineJobBinding::External{version:1,principal:source.principal(),grant_id:a.grant.id.clone(),grant_revision:a.grant.revision,assignment_revision:a.assignment.revision,protected_consent:source.protected_consent().cloned(),api_key_state_version_at_approval:match a.grant.binding{ExternalGrantBinding::ApiKey{api_key_state_version_at_approval,..}=>Some(api_key_state_version_at_approval),_=>None},authentication_expires_at:a.authentication_expires_at,service_ceiling:a.restrictions.service_ids.clone(),lease_id:lease_id.clone()})};
                let mut row=bson::to_document(&job).map_err(|_|refused())?;row.insert("external_authority",encode(&authority)?);
                db.collection::<Document>(machine_job::COLLECTION_NAME).insert_one(row).session(&mut *session).await?;Some(job)
            }else{existing.clone()};
            Ok(Admission{authority,job,pending:None,replayed:false})
        }.await;transactions::transaction_result(result)
    }).await.map_err(transactions::map_transaction_error)?;
    Ok(admitted)
}

pub async fn definitions(
    state: &AppState,
    source: &VerifiedExternalSource,
) -> AppResult<Vec<super::mcp_service::McpToolDefinition>> {
    if !source.discovery_possible() {
        return Ok(vec![]);
    }
    Ok(if visible(&state.db, source).await?.is_empty() {
        vec![]
    } else {
        super::machine_tools::external_definitions()
    })
}

/// Root MCP dispatch invokes this only for a verified external principal.
/// Discovery hints never authorize execution: every explicit call passes the
/// live source/grant transaction, managed readiness/funding gates and signed v3
/// Node capability negotiation in dispatch_authorized.
pub fn dispatch<'a>(
    state: &'a AppState,
    source: &'a VerifiedExternalSource,
    name: &'a str,
    mut arguments: Value,
) -> futures::future::BoxFuture<'a, AppResult<Value>> {
    Box::pin(async move { dispatch_authorized(state, source, name, &mut arguments).await })
}

pub(crate) async fn dispatch_authorized(
    state: &AppState,
    source: &VerifiedExternalSource,
    name: &str,
    arguments: &mut Value,
) -> AppResult<Value> {
    if !arguments.is_object() {
        return Err(invalid("Machine arguments must be an object"));
    }
    if serde_json::to_vec(arguments)
        .map_err(|_| invalid("Invalid Machine arguments"))?
        .len()
        > nyxid_machine::MAX_FRAME_BYTES - 512 * 1024
    {
        return Err(AppError::MachineLimitExceeded);
    }
    if name == "nyx__machine_list" {
        return super::machine_tools::list_page(
            "machines",
            visible(&state.db, source).await?,
            arguments,
            json!({"mode":"shared_legacy","output_retention":"runtime memory, at most one hour after completion"}),
        );
    }
    let operation = super::machine_tools::operation(name)
        .filter(|op| {
            matches!(
                op,
                Operation::Exec
                    | Operation::Job
                    | Operation::JobCancel
                    | Operation::ListFiles
                    | Operation::ReadFile
                    | Operation::WriteFile
                    | Operation::EditFile
            )
        })
        .ok_or(AppError::MachineNotAllowed)?;
    if operation == Operation::WriteFile {
        use base64::Engine;
        let content = arguments
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("content is required"))?;
        let limit = nyxid_machine::managed::MANAGED_FILE_WRITE_BYTES;
        let size = match arguments
            .get("encoding")
            .and_then(Value::as_str)
            .unwrap_or("text")
        {
            "text" => content.len(),
            "base64" => {
                if content.len() > limit.div_ceil(3) * 4 + 4 {
                    return Err(AppError::MachineLimitExceeded);
                }
                base64::engine::general_purpose::STANDARD
                    .decode(content)
                    .map_err(|_| invalid("Invalid base64 file content"))?
                    .len()
            }
            _ => return Err(invalid("encoding must be text or base64")),
        };
        if size > limit {
            return Err(AppError::MachineLimitExceeded);
        }
    }
    let node = resolve_node(
        &state.db,
        source,
        arguments["machine"].as_str().unwrap_or_default(),
    )
    .await?;
    if operation == Operation::Job && arguments.get("job_id").is_none() {
        return history(&state.db, source, &node.id, arguments).await;
    }
    let existing = if matches!(operation, Operation::Job | Operation::JobCancel) {
        let id = arguments["job_id"]
            .as_str()
            .filter(|s| Uuid::parse_str(s).is_ok())
            .ok_or_else(|| invalid("job_id must be a UUID"))?;
        let job=state.db.collection::<MachineJob>(machine_job::COLLECTION_NAME).find_one(doc!{"_id":id,"node_id":&node.id,"user_id":principal_user(&source.principal()),"binding.kind":"external"}).await?.ok_or(AppError::MachineJobNotFound)?;
        if !same_principal(external_binding(&job)?.0, &source.principal()) {
            return Err(AppError::MachineJobNotFound);
        }
        Some(job)
    } else {
        None
    };
    if operation == Operation::JobCancel {
        return cancel(state, source, &node, existing.as_ref().expect("job")).await;
    }
    if let Some(job) = &existing {
        // Metadata may cross runtime replacement; raw output must never be
        // requested from a successor daemon that does not own this job.
        authorize_history_job(&state.db, source, job).await?;
        if job.runtime_id
            != node
                .machine
                .as_ref()
                .map(|p| p.runtime_id.as_str())
                .unwrap_or_default()
            || job
                .finished_at
                .is_some_and(|at| at + Duration::hours(1) <= Utc::now())
        {
            return Ok(
                json!({"job":job_metadata(job),"output_available":false,"output_unavailable_reason":if job.runtime_id!=node.machine.as_ref().map(|p|p.runtime_id.as_str()).unwrap_or_default(){"runtime_replaced"}else{"retention_expired"}}),
            );
        }
        if job.state == "running" && external_binding(job)?.0 == &source.principal() {
            renew_job_with_source(state, source, job).await?;
        }
    }
    if operation == Operation::Exec {
        let command = arguments["command"]
            .as_str()
            .ok_or_else(|| invalid("command is required"))?;
        if command.is_empty()
            || command.len() > 32 * 1024
            || arguments["stdin"]
                .as_str()
                .is_some_and(|s| s.len() > 64 * 1024)
        {
            return Err(invalid("Command or stdin exceeds the input limit"));
        }
    }
    let admission = admit_request(
        state,
        source,
        &node,
        operation,
        arguments,
        existing.as_ref(),
    )
    .await?;
    if let Some(c) = admission.pending {
        return Ok(
            json!({"status":"confirmation_pending","confirmation_id":c.id,"acknowledgement_id":c.id,"expires_at":c.expires_at,"owner_only":true,"approval_url":format!("{}/assistant/machines?confirmation={}",state.config.frontend_url.trim_end_matches('/'),c.id)}),
        );
    }
    if admission.replayed {
        return Ok(
            json!({"status":"already_consumed","replayed":true,"job":admission.job.as_ref().map(job_metadata),"message":"This approval was already consumed; no action was repeated."}),
        );
    }
    if operation == Operation::Exec {
        let principal = source.principal();
        super::audit_service::log_async(
            state.db.clone(),
            Some(node.user_id.clone()),
            "machine_external_job_admitted".into(),
            Some(
                json!({"node_id":node.id,"job_id":admission.job.as_ref().map(|j|&j.id),"principal":principal,"grant_id":admission.authority.grant_id,"grant_revision":admission.authority.grant_revision,"runtime_id":admission.authority.runtime_id,"runtime_generation":admission.authority.runtime_generation,"services":admission.authority.services}),
            ),
            None,
            None,
            None,
            None,
        );

        let command = arguments["command"]
            .as_str()
            .ok_or_else(|| invalid("command is required"))?;
        if command.is_empty()
            || command.len() > 32 * 1024
            || arguments["stdin"]
                .as_str()
                .is_some_and(|s| s.len() > 64 * 1024)
        {
            return Err(invalid("Command or stdin exceeds the input limit"));
        }
        arguments["job_id"] = json!(admission.job.as_ref().expect("issued job").id);
        let available = super::machine_gateway_service::external_services(
            &state.db,
            &node.user_id,
            &admission
                .authority
                .services
                .iter()
                .map(|s| s.id.clone())
                .collect::<Vec<_>>(),
        )
        .await?;
        arguments["environment"] =
            serde_json::to_value(super::machine_gateway_service::environment(&available)?)
                .map_err(|_| refused())?;
    }
    if let Some(map) = arguments.as_object_mut() {
        map.remove("machine");
        map.remove("acknowledgement_id");
    }
    let response = send(
        state,
        &node,
        operation,
        arguments.clone(),
        admission.authority,
    )
    .await;
    match response {
        Ok(result) => {
            if let Some(job) = &admission.job
                && (operation == Operation::Job || operation == Operation::Exec)
                && matches!(
                    result["status"].as_str(),
                    Some("finished" | "failed" | "cancelled" | "interrupted")
                )
            {
                finish_observed(&state.db, &job.id, &node.id, &job.runtime_id, &result).await?;
            }
            Ok(super::machine_tools::bounded_result(result))
        }
        Err(error) => {
            if operation == Operation::Exec
                && let Some(job) = &admission.job
            {
                finish_terminal(&state.db, &job.id, &node.id, &job.runtime_id, "interrupted")
                    .await?;
            }
            Err(error)
        }
    }
}

async fn send(
    state: &AppState,
    node: &Node,
    operation: Operation,
    parameters: Value,
    authority: Box<ExternalAuthority>,
) -> AppResult<Value> {
    let secret =
        super::node_service::signing_secret_from_node(&state.encryption_keys, node).await?;
    let mut request = nyxid_machine::Request {
        version: nyxid_machine::authority::EXTERNAL_VERSION,
        request_id: Uuid::new_v4().to_string(),
        node_id: node.id.clone(),
        operation,
        parameters,
        timestamp: Utc::now().timestamp(),
        nonce: Uuid::new_v4().to_string(),
        signature: String::new(),
        authority: None,
        external_authority: Some(authority),
    };
    request.signature = nyxid_machine::signing::sign(&request, &secret);
    if serde_json::to_vec(&request).map_err(|_| refused())?.len() > nyxid_machine::MAX_FRAME_BYTES {
        return Err(AppError::MachineLimitExceeded);
    }
    Ok(state
        .node_dispatch
        .machine_request_with_node(request, node)
        .await?
        .result)
}

pub async fn finish_terminal(
    db: &Database,
    id: &str,
    node_id: &str,
    runtime: &str,
    state: &str,
) -> AppResult<()> {
    if !matches!(
        state,
        "finished" | "cancelled" | "interrupted" | "revoked" | "expired"
    ) {
        return Err(invalid("Invalid terminal job state"));
    }
    let now = Utc::now();
    db.collection::<Document>(machine_job::COLLECTION_NAME).update_one(doc!{"_id":id,"node_id":node_id,"runtime_id":runtime,"state":"running"},doc!{"$set":{"state":state,"stop_reason":state,"finished_at":bson::DateTime::from_chrono(now),"retain_until":bson::DateTime::from_chrono(now+Duration::days(HISTORY_DAYS))}}).await?;
    Ok(())
}

/// Signed Node results may add bounded terminal metadata but cannot change an
/// earlier revoked/cancelled/interrupted safety decision or its retention clock.
pub async fn finish_observed(
    db: &Database,
    id: &str,
    node_id: &str,
    runtime: &str,
    result: &Value,
) -> AppResult<()> {
    let state = match result.get("status").and_then(Value::as_str) {
        Some("cancelled") => "cancelled",
        Some("interrupted") => "interrupted",
        _ => "finished",
    };
    finish_terminal(db, id, node_id, runtime, state).await?;
    let mut metadata = doc! {};
    if let Some(code) = result
        .get("exit_code")
        .and_then(Value::as_i64)
        .and_then(|v| i32::try_from(v).ok())
    {
        metadata.insert("exit_code", code);
    }
    if let Some(duration) = result
        .get("duration_ms")
        .and_then(Value::as_i64)
        .filter(|v| *v >= 0 && *v <= 86_415_000)
    {
        metadata.insert("duration_ms", duration);
    }
    let reason = if result.get("timed_out") == Some(&Value::Bool(true)) {
        "timeout"
    } else {
        result
            .get("stop_reason")
            .and_then(Value::as_str)
            .filter(|reason| matches!(*reason, "completed" | "timeout" | "cancelled" | "signal"))
            .unwrap_or(state)
    };
    metadata.insert("stop_reason", reason);
    db.collection::<Document>(machine_job::COLLECTION_NAME)
        .update_one(
            doc! {"_id":id,"node_id":node_id,"runtime_id":runtime,"state":state},
            doc! {"$set":metadata},
        )
        .await?;
    Ok(())
}

fn job_metadata(job: &MachineJob) -> Value {
    let binding = match job.binding.as_ref() {
        Some(MachineJobBinding::External {
            principal,
            grant_id,
            grant_revision,
            ..
        }) => {
            json!({"kind":"external","principal":principal,"grant_id":grant_id,"grant_revision":grant_revision})
        }
        Some(MachineJobBinding::Assistant {
            agent_id,
            conversation_id,
            ..
        }) => json!({"kind":"assistant","agent_id":agent_id,"conversation_id":conversation_id}),
        None => json!({"kind":"assistant","legacy":true}),
    };
    json!({"id":job.id,"job_id":job.id,"node_id":job.node_id,"runtime_id":job.runtime_id,"runtime_generation":job.runtime_generation,"state":job.state,"exit_code":job.exit_code,"stop_reason":job.stop_reason,"duration_ms":job.duration_ms,"cancellation_requested_by":job.cancellation_requested_by,"cancellation_requested_at":job.cancellation_requested_at,"binding":binding,"services":job.services,"created_at":job.created_at,"expires_at":job.expires_at,"finished_at":job.finished_at,"retain_until":job.retain_until,"output_persisted":false})
}

async fn authorize_history_job(
    db: &Database,
    source: &VerifiedExternalSource,
    job: &MachineJob,
) -> AppResult<AdmittedGrant> {
    let (principal, grant, revision, _, _, _, _) = external_binding(job)?;
    if !same_principal(principal, &source.principal()) {
        return Err(AppError::MachineJobNotFound);
    }
    admit_read(db, source, &job.node_id, grant, revision).await
}

/// These read consumers still write the same live authority fence as renewal.
/// Retry only that database decision under the original caller and grant; Node
/// output/renewal dispatch remains outside the driver's bounded retry callback.
async fn admit_read(
    db: &Database,
    source: &VerifiedExternalSource,
    node_id: &str,
    grant_id: &str,
    grant_revision: i64,
) -> AppResult<AdmittedGrant> {
    let db = db.clone();
    let source = source.clone();
    let node_id = node_id.to_owned();
    let grant_id = grant_id.to_owned();
    db.client()
        .start_session()
        .await?
        .start_transaction()
        .write_concern(mongodb::options::WriteConcern::majority())
        .and_run2(async move |session| {
            transactions::transaction_result(
                grants::admit_in_session(
                    &db,
                    &source,
                    &node_id,
                    Some((&grant_id, grant_revision)),
                    session,
                )
                .await,
            )
        })
        .await
        .map_err(transactions::map_transaction_error)
}

fn page_filter(args: &Value) -> AppResult<(Document, i64)> {
    let limit = args
        .get("limit")
        .map(Value::as_i64)
        .unwrap_or(Some(20))
        .filter(|n| (1..=MAX_JOBS_PAGE).contains(n))
        .ok_or_else(|| invalid("Job page limit must be 1 to 50"))?;
    let mut filter = doc! {};
    let mut dates = doc! {};
    let mut from = None;
    let mut to = None;
    for (argument, operator) in [("from", "$gte"), ("to", "$lte")] {
        if let Some(value) = args.get(argument) {
            let at = value
                .as_str()
                .and_then(|v| DateTime::parse_from_rfc3339(v).ok())
                .map(|at| at.with_timezone(&Utc))
                .ok_or_else(|| invalid("Job dates must be RFC3339 timestamps"))?;
            dates.insert(operator, bson::DateTime::from_chrono(at));
            if argument == "from" {
                from = Some(at);
            } else {
                to = Some(at);
            }
        }
    }
    if from.zip(to).is_some_and(|(from, to)| from > to) {
        return Err(invalid("Job date range is reversed"));
    }
    if !dates.is_empty() {
        filter.insert("created_at", dates);
    }
    if let Some(state) = args.get("state").and_then(Value::as_str) {
        if !matches!(
            state,
            "running" | "finished" | "cancelled" | "interrupted" | "revoked" | "expired"
        ) {
            return Err(invalid("Unknown job state"));
        }
        filter.insert("state", state);
    }
    if let Some(cursor) = args.get("cursor").and_then(Value::as_str) {
        let (ms, id) = cursor
            .split_once(':')
            .ok_or_else(|| invalid("Invalid job cursor"))?;
        let ms = ms
            .parse::<i64>()
            .map_err(|_| invalid("Invalid job cursor"))?;
        if Uuid::parse_str(id).is_err() {
            return Err(invalid("Invalid job cursor"));
        }
        filter.insert("$and",vec![doc!{"$or":[{"created_at":{"$lt":bson::DateTime::from_millis(ms)}},{"created_at":bson::DateTime::from_millis(ms),"_id":{"$lt":id}}]}]);
    }
    filter.insert(
        "$or",
        vec![
            doc! {"retain_until":Bson::Null},
            doc! {"retain_until":{"$gt":bson::DateTime::now()}},
        ],
    );
    Ok((filter, limit))
}

pub async fn history(
    db: &Database,
    source: &VerifiedExternalSource,
    node_id: &str,
    args: &Value,
) -> AppResult<Value> {
    let grant = active_grant(db, source).await?;
    admit_read(db, source, node_id, &grant.id, grant.revision).await?;
    let (mut filter, limit) = page_filter(args)?;
    filter.extend(doc!{"node_id":node_id,"user_id":principal_user(&source.principal()),"binding.kind":"external","binding.grant_id":&grant.id,"binding.grant_revision":grant.revision});
    history_page(db, filter, limit).await
}
async fn history_page(db: &Database, filter: Document, limit: i64) -> AppResult<Value> {
    let mut rows: Vec<MachineJob> = db
        .collection(machine_job::COLLECTION_NAME)
        .find(filter)
        .sort(doc! {"created_at":-1,"_id":-1})
        .limit(limit + 1)
        .await?
        .try_collect()
        .await?;
    let more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let cursor = if more {
        rows.last()
            .map(|j| format!("{}:{}", j.created_at.timestamp_millis(), j.id))
    } else {
        None
    };
    Ok(
        json!({"jobs":rows.iter().map(job_metadata).collect::<Vec<_>>(),"next_cursor":cursor,"has_more":more,"output_persisted":false}),
    )
}

pub async fn owner_history(
    db: &Database,
    owner: &str,
    node_id: &str,
    args: &Value,
) -> AppResult<Value> {
    let node = super::node_service::get_node_by_id(db, node_id).await?;
    if node.as_ref().is_some_and(|node| node.user_id != owner) {
        return Err(AppError::MachineNotAllowed);
    }
    let (mut filter, limit) = page_filter(args)?;
    filter.extend(doc! {"node_id":node_id,"user_id":owner});
    let mut page = history_page(db, filter, limit).await?;
    let live = node.as_ref().is_some_and(|node| {
        node.is_active && node.machine.as_ref().is_some_and(|p| p.authority_v3())
    });
    let contexts: Vec<Document> = db
        .collection(access::CONTEXTS)
        .find(doc! {"node_id":node_id,"mode":"separated"})
        .projection(doc! {"agent_id":1})
        .await?
        .try_collect()
        .await?;
    if let Some(jobs) = page.get_mut("jobs").and_then(Value::as_array_mut) {
        for job in jobs {
            let same_runtime = node
                .as_ref()
                .and_then(|node| node.machine.as_ref())
                .is_some_and(|p| job["runtime_id"] == p.runtime_id);
            let supported_context = !contexts.iter().any(|row| {
                row.get_str("agent_id")
                    .is_ok_and(|agent| job["binding"]["agent_id"] == agent)
            });
            let output_fresh = job["finished_at"]
                .as_str()
                .and_then(|v| DateTime::parse_from_rfc3339(v).ok())
                .is_none_or(|at| at.with_timezone(&Utc) + Duration::hours(1) > Utc::now());
            job["output_query_available"] =
                json!(live && supported_context && same_runtime && output_fresh);
            job["output_availability"] =
                json!(
                    if live && supported_context && same_runtime && output_fresh {
                        "query_required"
                    } else {
                        "unavailable"
                    }
                );
            job["cancellation_available"] =
                json!(live && supported_context && job["state"] == "running");
        }
    }
    Ok(page)
}

async fn live_job_in_session(
    db: &Database,
    source: &VerifiedExternalSource,
    job: &MachineJob,
    node: &Node,
    session: &mut ClientSession,
) -> AppResult<(AdmittedGrant, Box<ExternalAuthority>)> {
    let (principal, grant, revision, assignment_revision, auth_until, ceiling, lease_id) =
        external_binding(job)?;
    if principal != &source.principal()
        || job.state != "running"
        || job.expires_at <= Utc::now()
        || auth_until <= Utc::now()
    {
        return Err(refused());
    }
    let a =
        grants::admit_in_session(db, source, &node.id, Some((grant, revision)), session).await?;
    if !a.assignment.capabilities.shell
        || a.assignment.revision != assignment_revision
        || node
            .machine
            .as_ref()
            .is_none_or(|p| p.runtime_id != job.runtime_id || !p.authority_v3())
    {
        return Err(refused());
    }
    let runtime =
        super::remote_machine_runtime::fence_execution_in_session(db, node, session).await?;
    if runtime.runtime_generation != job.runtime_generation {
        return Err(AppError::MachineAuthorityStale);
    }
    let lease=db.collection::<ExternalLease>(access::LEASES).find_one(doc!{"_id":lease_id,"kind":"external","job_id":&job.id,"external_authority.expires_at_ms":{"$gt":Utc::now().timestamp_millis()}}).session(&mut *session).await?.ok_or_else(refused)?;
    let mut authority = lease.external_authority;
    if authority.principal != wire_principal(principal)
        || authority.grant_id != grant
        || authority.grant_revision != revision
        || authority.node_id != job.node_id
        || authority.runtime_id != job.runtime_id
        || authority.runtime_generation != job.runtime_generation
        || authority.job_id.as_deref() != Some(job.id.as_str())
        || authority
            .oauth_consent
            .as_ref()
            .map(|c| (&c.consent_id, &c.consent_revision))
            != source
                .protected_consent()
                .map(|c| (&c.consent_id, &c.consent_revision))
        || authority.credential_epoch != runtime.credential_epoch
    {
        return Err(refused());
    }
    authority.auth_expires_at_ms = auth_until
        .min(a.authentication_expires_at)
        .timestamp_millis();
    authority.funding_expires_at_ms = runtime.funding_cutoff.map(|x| x.timestamp_millis());
    // No added services after issue time, even if the grant or source expands.
    authority.services.retain(|s| {
        ceiling.contains(&s.id)
            && a.restrictions.service_ids.contains(&s.id)
            && job
                .services
                .iter()
                .any(|d| d.id == s.id && d.slug == s.slug)
    });
    authority.expires_at_ms = (Utc::now().timestamp_millis() + nyxid_machine::authority::LEASE_MS)
        .min(authority.auth_expires_at_ms)
        .min(job.expires_at.timestamp_millis())
        .min(authority.funding_expires_at_ms.unwrap_or(i64::MAX));
    if authority.expires_at_ms <= Utc::now().timestamp_millis() {
        return Err(refused());
    }
    let touched=db.collection::<Document>(machine_job::COLLECTION_NAME).update_one(doc!{"_id":&job.id,"state":"running","binding.lease_id":lease_id,"runtime_id":&job.runtime_id},doc!{"$inc":{"machine_execution_fence":1_i64}}).session(session).await?;
    if touched.matched_count != 1 {
        return Err(refused());
    }
    Ok((a, authority))
}

pub async fn gateway_auth(db: &Database, job: &MachineJob) -> AppResult<crate::mw::auth::AuthUser> {
    let db_owned = db.clone();
    let identity = job.clone();
    // The source and execution fence are shared with admission and renewal.
    // A transient conflict retries live reads, never the downstream effect.
    let (job, node, authority) = db_owned.client().start_session().await?
        .start_transaction().write_concern(mongodb::options::WriteConcern::majority())
        .and_run2(async move |session| {
            let result: AppResult<(MachineJob, Node, Box<ExternalAuthority>)> = async {
                let job = db_owned.collection::<MachineJob>(machine_job::COLLECTION_NAME)
                    .find_one(doc! {"_id":&identity.id,"user_id":&identity.user_id,"node_id":&identity.node_id})
                    .session(&mut *session).await?.ok_or_else(refused)?;
                let node = db_owned.collection::<Node>(node::COLLECTION_NAME)
                    .find_one(doc! {"_id":&job.node_id}).session(&mut *session).await?.ok_or_else(refused)?;
                #[cfg(test)]
                tests::gateway_snapshot_loaded(&db_owned, &job.id).await;
                let source = source_for_job(&db_owned, &job).await?;
                let (_, authority) = live_job_in_session(&db_owned, &source, &job, &node, session).await?;
                Ok((job, node, authority))
            }.await;
            transactions::transaction_result(result)
        }).await.map_err(transactions::map_transaction_error)?;
    let ids: Vec<_> = authority.services.iter().map(|s| s.id.clone()).collect();
    crate::mw::auth::machine_external_gateway_auth_user(
        db,
        external_binding(&job)?.0,
        &node.id,
        &ids,
    )
    .await
}

async fn renew_job_with_source(
    state: &AppState,
    source: &VerifiedExternalSource,
    job: &MachineJob,
) -> AppResult<()> {
    let (principal, grant, revision, _, _, _, lease_id) = external_binding(job)?;
    if principal != &source.principal() {
        return Err(refused());
    }
    let node = super::node_service::get_node_by_id(&state.db, &job.node_id)
        .await?
        .ok_or_else(refused)?;
    let db = state.db.clone();
    let source = source.clone();
    let job = job.clone();
    let grant = grant.to_owned();
    let lease_id = lease_id.to_owned();
    let node_tx = node.clone();
    let authority=db.client().start_session().await?.start_transaction().write_concern(mongodb::options::WriteConcern::majority()).and_run2(async move |session|{
        let result:AppResult<Box<ExternalAuthority>>=async {
            let a=grants::admit_in_session(&db,&source,&job.node_id,Some((&grant,revision)),session).await?;
            // A still-live execution can accept a new authentication deadline
            // only from its exact original source child (OAuth same client and
            // protected consent). Shared sibling reads do not reach this path.
            let changed=db.collection::<Document>(machine_job::COLLECTION_NAME).update_one(doc!{"_id":&job.id,"state":"running","binding.lease_id":&lease_id,"binding.authentication_expires_at":{"$gt":bson::DateTime::now()},"expires_at":{"$gt":bson::DateTime::now()}},doc!{"$max":{"binding.authentication_expires_at":bson::DateTime::from_chrono(a.authentication_expires_at)}}).session(&mut *session).await?;
            if changed.matched_count!=1{return Err(refused());}
            let job=db.collection::<MachineJob>(machine_job::COLLECTION_NAME).find_one(doc!{"_id":&job.id}).session(&mut *session).await?.ok_or_else(refused)?;
            let (_,authority)=live_job_in_session(&db,&source,&job,&node_tx,session).await?;
            db.collection::<Document>(access::LEASES).update_one(doc!{"_id":&lease_id,"external_authority.expires_at_ms":{"$gt":Utc::now().timestamp_millis()}},doc!{"$set":{"external_authority":encode(&authority)?}}).session(&mut *session).await?;
            db.collection::<Document>(machine_job::COLLECTION_NAME).update_one(doc!{"_id":&job.id,"state":"running"},doc!{"$set":{"external_authority":encode(&authority)?}}).session(session).await?;
            Ok(authority)
        }.await;transactions::transaction_result(result)
    }).await.map_err(transactions::map_transaction_error)?;
    let result = send(
        state,
        &node,
        Operation::AuthorityRenew,
        json!({}),
        authority,
    )
    .await?;
    if result["accepted"] != true {
        return Err(AppError::MachineAuthorityStale);
    }
    Ok(())
}

/// Narrow cancellation checks actual live source identity without requiring a
/// grant that may already have been revoked. It cannot read output or renew.
async fn valid_stop_source(db: &Database, source: &VerifiedExternalSource) -> AppResult<()> {
    if source.authentication_expires_at() <= Utc::now() {
        return Err(refused());
    }
    let principal = source.principal();
    let user = db
        .collection::<Document>(crate::models::user::COLLECTION_NAME)
        .find_one(
            doc! {"_id":principal_user(&principal),"is_active":true,"user_type":{"$ne":"org"}},
        )
        .await?;
    if user.is_none() {
        return Err(refused());
    }
    match principal {
        ExternalPrincipalRef::ApiKey {
            user_id,
            api_key_id,
            api_key_credential_id,
        } => {
            let key = super::key_service::get_api_key(db, &user_id, &api_key_id).await?;
            VerifiedExternalSource::from_verified_api_key(&key, api_key_credential_id.as_deref())?;
            if let Some(child) = api_key_credential_id {
                let mut session = db.client().start_session().await?;
                session.start_transaction().await?;
                let valid = super::machine_external_source_service::fence_child_in_session(
                    db,
                    &user_id,
                    &api_key_id,
                    &child,
                    &mut session,
                )
                .await;
                match valid {
                    Ok(_) => session.commit_transaction().await?,
                    Err(error) => {
                        let _ = session.abort_transaction().await;
                        return Err(error);
                    }
                }
            }
        }
        ExternalPrincipalRef::OAuth {
            user_id,
            oauth_client_id,
        } => {
            let consent = source.protected_consent().ok_or_else(refused)?;
            if db
                .collection::<Document>(crate::models::oauth_client::COLLECTION_NAME)
                .find_one(doc! {"_id":&oauth_client_id,"is_active":true})
                .await?
                .is_none()
            {
                return Err(refused());
            }
            if db.collection::<Document>(crate::models::consent::COLLECTION_NAME).find_one(doc!{"_id":&consent.consent_id,"user_id":&user_id,"client_id":&oauth_client_id,"revision":&consent.consent_revision,"$or":[{"expires_at":Bson::Null},{"expires_at":{"$gt":bson::DateTime::now()}}]}).await?.is_none(){return Err(refused());}
        }
    }
    Ok(())
}
async fn cancel(
    state: &AppState,
    source: &VerifiedExternalSource,
    node: &Node,
    job: &MachineJob,
) -> AppResult<Value> {
    valid_stop_source(&state.db, source).await?;
    if !same_principal(external_binding(job)?.0, &source.principal()) {
        return Err(AppError::MachineJobNotFound);
    }
    let row = state
        .db
        .collection::<Document>(machine_job::COLLECTION_NAME)
        .find_one(doc! {"_id":&job.id})
        .await?
        .ok_or(AppError::MachineJobNotFound)?;
    let mut authority: Box<ExternalAuthority> =
        bson::from_bson(row.get("external_authority").cloned().ok_or_else(refused)?)
            .map_err(|_| refused())?;
    authority.expires_at_ms = Utc::now().timestamp_millis() + nyxid_machine::authority::LEASE_MS;
    let lease_id = authority.lease_id.clone();
    let mut session = state.db.client().start_session().await?;
    session.start_transaction().await?;
    enqueue_lease_in_session(&state.db, &authority, "job_cancelled", &mut session).await?;
    let requested_at = Utc::now();
    let requester = MachineCancellationRequester::External {
        principal: source.principal(),
    };
    state.db.collection::<Document>(machine_job::COLLECTION_NAME).update_one(doc!{"_id":&job.id,"node_id":&node.id,"runtime_id":&job.runtime_id,"state":"running"},doc!{"$set":{"state":"cancelled","finished_at":bson::DateTime::from_chrono(requested_at),"retain_until":bson::DateTime::from_chrono(requested_at+Duration::days(HISTORY_DAYS)),"cancellation_requested_by":encode(&requester)?,"cancellation_requested_at":bson::DateTime::from_chrono(requested_at)}}).session(&mut session).await?;
    state
        .db
        .collection::<Document>(access::LEASES)
        .delete_one(doc! {"_id":&lease_id})
        .session(&mut session)
        .await?;
    session.commit_transaction().await?;
    // Durable outbox owns retries; this request only attempts the immediate
    // low-latency stop. The result contains no output under any circumstance.
    let accepted = if node
        .machine
        .as_ref()
        .is_some_and(|p| p.runtime_id == job.runtime_id)
    {
        send(
            state,
            node,
            Operation::AuthorityRevoke,
            json!({"selector":"lease"}),
            authority,
        )
        .await
        .is_ok_and(|v| v["accepted"] == true)
    } else {
        false
    };
    Ok(
        json!({"job_id":job.id,"status":"cancelled","cancellation_acknowledged":accepted,"stop_acknowledged":false}),
    )
}

#[cfg(test)]
pub async fn owner_confirmations(
    db: &Database,
    owner: &str,
    node_id: Option<&str>,
) -> AppResult<Value> {
    owner_confirmations_page(db, owner, node_id, None, None).await
}

pub async fn owner_confirmations_page(
    db: &Database,
    owner: &str,
    node_id: Option<&str>,
    cursor: Option<&str>,
    limit: Option<u32>,
) -> AppResult<Value> {
    let now = bson::DateTime::now();
    let mut filter = doc! {"user_id":owner,"state":{"$in":["pending","approved"]},"expires_at":{"$gt":now},"authentication_expires_at":{"$gt":now}};
    if let Some(node) = node_id {
        filter.insert("node_id", node);
    }
    if let Some(cursor) = cursor {
        let (at, id) = decode_confirmation_cursor(cursor)?;
        filter.insert(
            "$or",
            bson::bson!([
                {"created_at":{"$lt":bson::DateTime::from_chrono(at)}},
                {"created_at":bson::DateTime::from_chrono(at),"_id":{"$lt":id}},
            ]),
        );
    }
    let limit = i64::from(limit.unwrap_or(100).clamp(1, 100));
    let mut rows: Vec<ExternalConfirmation> = db
        .collection(execution::CONFIRMATIONS)
        .find(filter)
        .sort(doc! {"created_at":-1,"_id":-1})
        .limit(limit + 1)
        .max_time(std::time::Duration::from_secs(3))
        .await?
        .try_collect()
        .await?;
    let more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let next_cursor = more.then(|| rows.last()).flatten().map(|row| {
        format!(
            "{}|{}",
            row.created_at
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            row.id
        )
    });
    Ok(
        json!({"confirmations":rows.iter().map(confirmation_metadata).collect::<Vec<_>>(),"next_cursor":next_cursor}),
    )
}

fn decode_confirmation_cursor(cursor: &str) -> AppResult<(DateTime<Utc>, String)> {
    let bad = || AppError::BadRequest("machine_confirmation_cursor_invalid".into());
    if cursor.len() > 80 {
        return Err(bad());
    }
    let (at, id) = cursor.split_once('|').ok_or_else(bad)?;
    let at = DateTime::parse_from_rfc3339(at)
        .map_err(|_| bad())?
        .with_timezone(&Utc);
    let id = Uuid::parse_str(id).map_err(|_| bad())?.to_string();
    Ok((at, id))
}

fn confirmation_metadata(c: &ExternalConfirmation) -> Value {
    json!({"id":c.id,"node_id":c.node_id,"principal":c.principal,"grant_id":c.grant_id,"grant_revision":c.grant_revision,"assignment_revision":c.assignment_revision,"runtime_id":c.runtime_id,"runtime_generation":c.runtime_generation,"operation":c.operation,"parameters_digest":c.parameters_digest,"safe_parameters":c.safe_parameters,"review_parameters_available":!c.safe_parameters.is_empty(),"review_content_available":false,"state":c.state,"created_at":c.created_at,"expires_at":c.expires_at,"decided_at":c.decided_at,"owner_only":true})
}

pub async fn owner_decide(db: &Database, owner: &str, id: &str, approve: bool) -> AppResult<Value> {
    let db = db.clone();
    let owner = owner.to_owned();
    let id = id.to_owned();
    let row = db.client().start_session().await?.start_transaction()
        .write_concern(mongodb::options::WriteConcern::majority())
        .and_run2(async move |session| {
            let result: AppResult<ExternalConfirmation> = async {
                // Refresh time on transaction retry. An earlier snapshot of wall
                // time may not approve a source that expired during contention.
                let now = Utc::now();
                let c = db.collection::<ExternalConfirmation>(execution::CONFIRMATIONS)
                    .find_one(doc! {"_id":&id,"user_id":&owner,"state":"pending","expires_at":{"$gt":bson::DateTime::from_chrono(now)}})
                    .session(&mut *session).await?.ok_or(AppError::MachineConfirmationDeclined)?;
                let source = VerifiedExternalSource::from_persisted_confirmation(&c)?;
                let node = db.collection::<Node>(node::COLLECTION_NAME)
                    .find_one(doc! {"_id":&c.node_id,"user_id":&owner,"is_active":true,"machine.runtime_id":&c.runtime_id})
                    .session(&mut *session).await?.ok_or_else(refused)?;
                let admitted = grants::admit_in_session(&db, &source, &c.node_id,
                    Some((&c.grant_id, c.grant_revision)), session).await?;
                if admitted.assignment.revision != c.assignment_revision {
                    return Err(refused());
                }
                let runtime = super::remote_machine_runtime::fence_execution_in_session(&db, &node, session).await?;
                if runtime.runtime_generation != c.runtime_generation || runtime.credential_epoch != c.credential_epoch {
                    return Err(AppError::MachineAuthorityStale);
                }
                let expires_at = (now + Duration::seconds(super::assistant_acknowledgement_service::ACTION_SECONDS))
                    .min(admitted.authentication_expires_at)
                    .min(runtime.funding_cutoff.unwrap_or(DateTime::<Utc>::MAX_UTC));
                if expires_at <= Utc::now() { return Err(AppError::MachineConfirmationDeclined); }
                db.collection::<ExternalConfirmation>(execution::CONFIRMATIONS)
                    .find_one_and_update(doc! {"_id":&id,"user_id":&owner,"state":"pending","expires_at":{"$gt":bson::DateTime::now()}},
                        doc! {"$set":{"state":if approve {"approved"} else {"declined"},"decided_at":bson::DateTime::from_chrono(now),"expires_at":bson::DateTime::from_chrono(expires_at)}})
                    .return_document(ReturnDocument::After).session(session).await?
                    .ok_or(AppError::MachineConfirmationDeclined)
            }.await;
            transactions::transaction_result(result)
        }).await.map_err(transactions::map_transaction_error)?;
    Ok(confirmation_metadata(&row))
}

fn selector_filter(selector: &ExternalRevocationSelector, prefix: &str) -> Document {
    let field = |suffix: &str| format!("{prefix}{suffix}");
    match selector {
        ExternalRevocationSelector::OAuthPrincipal {
            user_id,
            oauth_client_id,
        } => {
            doc! {field("principal.kind"):"oauth",field("principal.user_id"):user_id,field("principal.oauth_client_id"):oauth_client_id}
        }
        ExternalRevocationSelector::ApiKeyPrincipal {
            user_id,
            api_key_id,
        } => {
            doc! {field("principal.kind"):"api_key",field("principal.user_id"):user_id,field("principal.api_key_id"):api_key_id}
        }
        ExternalRevocationSelector::Child {
            user_id,
            api_key_id,
            api_key_credential_id,
        } => {
            doc! {field("principal.kind"):"api_key",field("principal.user_id"):user_id,field("principal.api_key_id"):api_key_id,field("principal.api_key_credential_id"):api_key_credential_id}
        }
        ExternalRevocationSelector::Grant {
            user_id,
            grant_id,
            through_revision,
        } => {
            doc! {field("principal.user_id"):user_id,field("grant_id"):grant_id,field("grant_revision"):{"$lte":through_revision}}
        }
    }
}

/// Called by source and grant mutations BEFORE their transaction commits. This
/// is the shared Machine revocation outbox, not an independent cancellation log.
pub async fn enqueue_external_in_session(
    db: &Database,
    selector: &ExternalRevocationSelector,
    source_fence: i64,
    reason: &str,
    session: &mut ClientSession,
) -> AppResult<()> {
    if source_fence < 0 || reason.is_empty() || reason.len() > 100 {
        return Err(invalid("Invalid external revocation"));
    }
    let selector_value = serde_json::to_value(selector).map_err(|_| refused())?;
    let dedupe = super::assistant_acknowledgement_service::arguments_digest(
        &json!({"selector":selector_value,"source_fence":source_fence,"reason":reason}),
    );
    db.collection::<Document>(access::OUTBOX).update_one(doc!{"kind":"external","dedupe":&dedupe},doc!{"$setOnInsert":{"_id":Uuid::new_v4().to_string(),"kind":"external","selector":encode(selector)?,"source_fence":source_fence,"reason":reason,"pending":true,"claim_until":bson::DateTime::from_millis(0),"created_at":bson::DateTime::now(),"dedupe":dedupe}}).upsert(true).session(&mut *session).await?;
    let mut filter = selector_filter(selector, "");
    filter.insert("state", doc! {"$in":["pending","approved"]});
    db.collection::<Document>(execution::CONFIRMATIONS)
        .update_many(
            filter,
            doc! {"$set":{"state":"revoked","decided_at":bson::DateTime::now()}},
        )
        .session(session)
        .await?;
    Ok(())
}
async fn enqueue_lease_in_session(
    db: &Database,
    authority: &ExternalAuthority,
    reason: &str,
    session: &mut ClientSession,
) -> AppResult<()> {
    db.collection::<Document>(access::OUTBOX).update_one(doc!{"kind":"external_lease","lease_id":&authority.lease_id},doc!{"$setOnInsert":{"_id":Uuid::new_v4().to_string(),"kind":"external_lease","lease_id":&authority.lease_id,"node_id":&authority.node_id,"external_authority":encode(authority)?,"reason":reason,"pending":true,"claim_until":bson::DateTime::from_millis(0),"created_at":bson::DateTime::now()}}).upsert(true).session(session).await?;
    Ok(())
}

pub async fn revoke_outbox(state: &AppState, row: Document) -> AppResult<()> {
    let id = row.get_str("_id").map_err(|_| refused())?.to_owned();
    let claim = Uuid::new_v4().to_string();
    let Some(row)=state.db.collection::<Document>(access::OUTBOX).find_one_and_update(doc!{"_id":&id,"pending":true,"claim_until":{"$lte":bson::DateTime::now()}},doc!{"$set":{"claim":&claim,"claim_until":bson::DateTime::from_chrono(Utc::now()+Duration::seconds(10))}}).return_document(ReturnDocument::After).await? else{return Ok(());};
    let mut complete = true;
    let mut outcome = "expanded_to_lease_stops";
    if row.get_str("kind").ok() == Some("external") {
        let selector: ExternalRevocationSelector =
            bson::from_bson(row.get("selector").cloned().ok_or_else(refused)?)
                .map_err(|_| refused())?;
        let mut filter = selector_filter(&selector, "external_authority.");
        filter.insert("kind", "external");
        let leases: Vec<ExternalLease> = state
            .db
            .collection(access::LEASES)
            .find(filter)
            .limit(100)
            .await?
            .try_collect()
            .await?;
        complete = leases.len() < 100;
        for lease in leases {
            let mut session = state.db.client().start_session().await?;
            session.start_transaction().await?;
            enqueue_lease_in_session(
                &state.db,
                &lease.external_authority,
                "source_revoked",
                &mut session,
            )
            .await?;
            state
                .db
                .collection::<Document>(access::LEASES)
                .delete_one(doc! {"_id":&lease.id})
                .session(&mut session)
                .await?;
            if let Some(job) = &lease.job_id {
                let requested_at = Utc::now();
                let requester = MachineCancellationRequester::System {
                    reason: MachineSystemCancellationReason::SourceRevoked,
                };
                state.db.collection::<Document>(machine_job::COLLECTION_NAME).update_one(doc!{"_id":job,"state":"running"},doc!{"$set":{"state":"revoked","finished_at":bson::DateTime::from_chrono(requested_at),"retain_until":bson::DateTime::from_chrono(requested_at+Duration::days(HISTORY_DAYS)),"cancellation_requested_by":encode(&requester)?,"cancellation_requested_at":bson::DateTime::from_chrono(requested_at)}}).session(&mut session).await?;
            }
            session.commit_transaction().await?;
        }
    } else if row.get_str("kind").ok() == Some("external_lease") {
        outcome = "original_runtime_unavailable";
        let mut authority: Box<ExternalAuthority> =
            bson::from_bson(row.get("external_authority").cloned().ok_or_else(refused)?)
                .map_err(|_| refused())?;
        let original_deadline = authority.expires_at_ms;
        let node = super::node_service::get_node_by_id(&state.db, &authority.node_id).await?;
        // A changed runtime cannot own this process. For a live same-runtime
        // node, push the exact original lease and retain retries until its
        // bounded signed deadline when no acknowledgement is available.
        if let Some(node) = node
            && node
                .machine
                .as_ref()
                .is_some_and(|p| p.runtime_id == authority.runtime_id)
        {
            authority.expires_at_ms =
                Utc::now().timestamp_millis() + nyxid_machine::authority::LEASE_MS;
            let acknowledged = send(
                state,
                &node,
                Operation::AuthorityRevoke,
                json!({"selector":"lease"}),
                authority,
            )
            .await
            .is_ok_and(|v| v["accepted"] == true);
            complete = acknowledged || original_deadline <= Utc::now().timestamp_millis();
            outcome = if acknowledged {
                "node_acknowledged"
            } else {
                "signed_lease_elapsed"
            };
        }
    } else if row.get_str("kind").ok() == Some("owner_job") {
        let job_id = row.get_str("job_id").map_err(|_| refused())?;
        let node_id = row.get_str("node_id").map_err(|_| refused())?;
        let owner = row.get_str("user_id").map_err(|_| refused())?;
        let job = state
            .db
            .collection::<MachineJob>(machine_job::COLLECTION_NAME)
            .find_one(doc! {"_id":job_id,"node_id":node_id,"user_id":owner})
            .await?;
        let node = super::node_service::get_node_by_id(&state.db, node_id).await?;
        outcome = "original_runtime_unavailable";
        if let (Some(job), Some(node)) = (job, node)
            && node
                .machine
                .as_ref()
                .is_some_and(|p| p.runtime_id == job.runtime_id)
        {
            let acknowledged = send_owner_job(state, &node, &job, Operation::JobCancel, json!({}))
                .await
                .is_ok_and(|value| value["cancel_requested"] == true);
            complete = acknowledged || job.expires_at <= Utc::now();
            outcome = if acknowledged {
                "node_acknowledged"
            } else {
                "execution_deadline_elapsed"
            };
        }
    } else {
        return Err(invalid("Unknown external revocation kind"));
    }
    state
        .db
        .collection::<Document>(access::OUTBOX)
        .update_one(
            doc! {"_id":&id,"claim":&claim,"pending":true},
            if complete {
                let mut completed = doc!{"pending":false,"completed_at":bson::DateTime::now(),"completion_evidence":outcome};
                if outcome == "node_acknowledged" { completed.insert("acknowledged_at", bson::DateTime::now()); }
                doc! {"$set":completed}
            } else {
                doc! {"$set":{"claim_until":bson::DateTime::from_millis(0)}}
            },
        )
        .await?;
    Ok(())
}

/// Runs from the existing Machine maintenance loop. Expiry cannot resurrect
/// jobs; the bounded retained metadata is independent of execution leases.
pub async fn maintenance(state: &AppState) -> AppResult<()> {
    let leases = state
        .db
        .collection::<ExternalLease>(access::LEASES)
        .find(doc! {"kind":"external"})
        .batch_size(128)
        .await?;
    leases
        .map_err(AppError::from)
        .try_for_each_concurrent(64, |lease| async move {
            if lease.external_authority.expires_at_ms <= Utc::now().timestamp_millis() {
                if let Some(job) = &lease.job_id {
                    finish_terminal(
                        &state.db,
                        job,
                        &lease.node_id,
                        &lease.external_authority.runtime_id,
                        "expired",
                    )
                    .await?;
                }
                state
                    .db
                    .collection::<Document>(access::LEASES)
                    .delete_one(doc! {"_id":&lease.id})
                    .await?;
                return Ok::<(), AppError>(());
            }
            let Some(job_id) = &lease.job_id else {
                return Ok(());
            };
            let Some(job) = state
                .db
                .collection::<MachineJob>(machine_job::COLLECTION_NAME)
                .find_one(doc! {"_id":job_id,"state":"running"})
                .await?
            else {
                return Ok(());
            };
            if let Ok(source) = source_for_job(&state.db, &job).await {
                let _ = tokio::time::timeout(
                    std::time::Duration::from_secs(2),
                    renew_job_with_source(state, &source, &job),
                )
                .await;
            }
            Ok(())
        })
        .await?;
    let now = Utc::now();
    state.db.collection::<Document>(machine_job::COLLECTION_NAME).update_many(doc!{"state":"running","expires_at":{"$lte":bson::DateTime::from_chrono(now)}},doc!{"$set":{"state":"expired","stop_reason":"execution_deadline","finished_at":bson::DateTime::from_chrono(now),"retain_until":bson::DateTime::from_chrono(now+Duration::days(HISTORY_DAYS))}}).await?;
    // Upgrade safe legacy metadata in bounded batches. Missing completion time
    // starts a fresh retention window now; never infer an earlier deletion date.
    let legacy:Vec<Document>=state.db.collection(machine_job::COLLECTION_NAME)
        .find(doc!{"state":{"$in":["finished","cancelled","interrupted","revoked","expired"]},"retain_until":Bson::Null})
        .projection(doc!{"_id":1}).sort(doc!{"_id":1}).limit(100).await?.try_collect().await?;
    let ids: Vec<&str> = legacy
        .iter()
        .filter_map(|row| row.get_str("_id").ok())
        .collect();
    if !ids.is_empty() {
        state.db.collection::<Document>(machine_job::COLLECTION_NAME).update_many(
            doc!{"_id":{"$in":ids},"state":{"$ne":"running"},"retain_until":Bson::Null},
            vec![doc!{"$set":{"finished_at":{"$ifNull":["$finished_at",bson::DateTime::from_chrono(now)]},"retain_until":{"$dateAdd":{"startDate":{"$ifNull":["$finished_at",bson::DateTime::from_chrono(now)]},"unit":"day","amount":HISTORY_DAYS}}}}]
        ).await?;
    }
    state.db.collection::<Document>(execution::CONFIRMATIONS).update_many(doc!{"state":{"$in":["pending","approved"]},"expires_at":{"$lte":bson::DateTime::from_chrono(now)}},doc!{"$set":{"state":"expired"}}).await?;
    Ok(())
}

#[cfg(test)]
#[path = "machine_external_execution_tests.rs"]
mod tests;

#[cfg(test)]
pub async fn owner_grants(
    db: &Database,
    owner: &str,
    node_id: &str,
) -> AppResult<Vec<crate::models::machine_external_grant::MachineExternalGrant>> {
    let node = super::node_service::get_node_by_id(db, node_id).await?;
    // An allocation owns its stable identity before bootstrap creates the
    // Node, and retains metadata after Node cleanup. When both records exist,
    // neither may contradict the owner; a stale/mismatched Node is not an ACL
    // fallback around the authoritative managed allocation.
    let managed = db
        .collection::<crate::models::remote_machine::ManagedMachine>(
            crate::models::remote_machine::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":node_id})
        .await?;
    if node.as_ref().is_some_and(|node| node.user_id != owner)
        || managed
            .as_ref()
            .is_some_and(|machine| machine.owner_id != owner)
    {
        return Err(AppError::MachineNotAllowed);
    }
    if node.is_none() && managed.is_none() {
        return Err(AppError::MachineJobNotFound);
    }
    let assignment = format!("policy.assignments.{node_id}");
    Ok(db
        .collection(crate::models::machine_external_grant::COLLECTION_NAME)
        .find(doc! {"user_id":owner,assignment:{"$exists":true}})
        .sort(doc! {"created_at":-1,"_id":-1})
        .limit(100)
        .await?
        .try_collect()
        .await?)
}

async fn owner_bound_job(
    db: &Database,
    owner: &str,
    node_id: &str,
    id: &str,
) -> AppResult<(Node, MachineJob)> {
    let node = super::node_service::get_node_by_id(db, node_id)
        .await?
        .filter(|n| n.user_id == owner)
        .ok_or(AppError::MachineJobNotFound)?;
    let job=db.collection::<MachineJob>(machine_job::COLLECTION_NAME).find_one(doc!{"_id":id,"node_id":node_id,"user_id":owner,"$or":[{"retain_until":Bson::Null},{"retain_until":{"$gt":bson::DateTime::now()}}]}).await?.ok_or(AppError::MachineJobNotFound)?;
    Ok((node, job))
}

async fn require_owner_job_support(db: &Database, node: &Node, job: &MachineJob) -> AppResult<()> {
    if node.machine.as_ref().is_none_or(|p| !p.authority_v3()) {
        return Err(AppError::MachineAuthorityUnsupported);
    }
    if !matches!(job.binding, Some(MachineJobBinding::External { .. }))
        && db
            .collection::<Document>(access::CONTEXTS)
            .find_one(doc! {"node_id":&node.id,"agent_id":&job.agent_id,"mode":"separated"})
            .projection(doc! {"_id":1})
            .await?
            .is_some()
    {
        return Err(AppError::MachineAuthorityUnsupported);
    }
    Ok(())
}

/// Human management uses a signed, narrow owner operation, not an invented
/// OAuth/key/Assistant principal. It cannot start or extend execution.
async fn send_owner_job(
    state: &AppState,
    node: &Node,
    job: &MachineJob,
    operation: Operation,
    mut parameters: Value,
) -> AppResult<Value> {
    if !matches!(operation, Operation::Job | Operation::JobCancel) {
        return Err(AppError::MachineNotAllowed);
    }
    require_owner_job_support(&state.db, node, job).await?;
    if node
        .machine
        .as_ref()
        .is_none_or(|p| p.runtime_id != job.runtime_id)
    {
        return Err(AppError::Conflict(
            "machine_job_output_unavailable: the original runtime ended".into(),
        ));
    }
    parameters["job_id"] = json!(job.id);
    parameters["owner_job_access"] =
        serde_json::to_value(nyxid_machine::authority::OwnerJobAccess {
            owner_id: node.user_id.clone(),
            node_id: node.id.clone(),
            runtime_id: job.runtime_id.clone(),
            job_id: job.id.clone(),
            expires_at_ms: Utc::now().timestamp_millis() + nyxid_machine::authority::LEASE_MS,
        })
        .map_err(|_| refused())?;
    let secret =
        super::node_service::signing_secret_from_node(&state.encryption_keys, node).await?;
    let mut request = nyxid_machine::Request {
        version: 1,
        authority: None,
        external_authority: None,
        request_id: Uuid::new_v4().to_string(),
        node_id: node.id.clone(),
        operation,
        parameters,
        timestamp: Utc::now().timestamp(),
        nonce: Uuid::new_v4().to_string(),
        signature: String::new(),
    };
    request.signature = nyxid_machine::signing::sign(&request, &secret);
    Ok(state
        .node_dispatch
        .machine_request_with_node(request, node)
        .await?
        .result)
}

pub async fn owner_output(
    state: &AppState,
    owner: &str,
    node_id: &str,
    id: &str,
    offset: u64,
    stderr_offset: u64,
) -> AppResult<Value> {
    if offset > 1 << 32 || stderr_offset > 1 << 32 {
        return Err(invalid("Output offset exceeds the bounded job output"));
    }
    let (node, job) = owner_bound_job(&state.db, owner, node_id, id).await?;
    if !node.is_active
        || job
            .finished_at
            .is_some_and(|time| time + Duration::hours(1) <= Utc::now())
    {
        return Err(AppError::Conflict(
            "machine_job_output_unavailable: output expired or Machine was deleted".into(),
        ));
    }
    owner_output_fence(&state.db, &node, &job).await?;
    let result = send_owner_job(
        state,
        &node,
        &job,
        Operation::Job,
        json!({"output_offset":offset,"stderr_offset":stderr_offset,"wait_secs":0}),
    )
    .await?;
    // Delete/takeover can commit while the Node returns its in-memory output.
    // Fence the result immediately before exposing it to the human response.
    owner_output_fence(&state.db, &node, &job).await?;
    if result["status"] == "finished" {
        finish_observed(&state.db, &job.id, &node.id, &job.runtime_id, &result).await?;
    }
    Ok(result)
}

async fn owner_output_fence(db: &Database, node: &Node, job: &MachineJob) -> AppResult<()> {
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .write_concern(mongodb::options::WriteConcern::majority())
        .await?;
    let result:AppResult<()>=async {
        let touched=db.collection::<Document>(node::COLLECTION_NAME).update_one(doc!{"_id":&node.id,"user_id":&job.user_id,"is_active":true,"machine.runtime_id":&job.runtime_id},doc!{"$inc":{"machine_owner_output_fence":1_i64}}).session(&mut session).await?;
        if touched.matched_count!=1{return Err(AppError::MachineNotAllowed);}
        if super::remote_machine_runtime::is_managed(node) {
            let generation=job.runtime_generation.and_then(|v|i64::try_from(v).ok()).ok_or(AppError::MachineAuthorityStale)?;
            let touched=db.collection::<Document>(crate::models::remote_machine::COLLECTION_NAME).update_one(doc!{"_id":&node.id,"owner_id":&job.user_id,"runtime_generation":generation,"resource_state":{"$nin":["deleting","deleted"]}},doc!{"$inc":{"machine_owner_output_fence":1_i64}}).session(&mut session).await?;
            if touched.matched_count!=1{return Err(AppError::MachineNotAllowed);}
        }
        Ok(())
    }.await;
    match result {
        Ok(()) => {
            session.commit_transaction().await?;
            Ok(())
        }
        Err(error) => {
            let _ = session.abort_transaction().await;
            Err(error)
        }
    }
}

pub async fn owner_cancel(
    state: &AppState,
    owner: &str,
    node_id: &str,
    id: &str,
) -> AppResult<Value> {
    let (node, job) = owner_bound_job(&state.db, owner, node_id, id).await?;
    require_owner_job_support(&state.db, &node, &job).await?;
    if job.state != "running" {
        return Ok(json!({"job_id":id,"status":job.state,"stop_acknowledged":false}));
    }
    let mut session = state.db.client().start_session().await?;
    session
        .start_transaction()
        .write_concern(mongodb::options::WriteConcern::majority())
        .await?;
    let requested_at = Utc::now();
    let requester = MachineCancellationRequester::HumanOwner {
        user_id: owner.to_owned(),
    };
    let changed=state.db.collection::<Document>(machine_job::COLLECTION_NAME).update_one(doc!{"_id":id,"user_id":owner,"node_id":node_id,"state":"running"},doc!{"$set":{"state":"cancelled","finished_at":bson::DateTime::from_chrono(requested_at),"retain_until":bson::DateTime::from_chrono(requested_at+Duration::days(HISTORY_DAYS)),"cancellation_requested_by":encode(&requester)?,"cancellation_requested_at":bson::DateTime::from_chrono(requested_at)}}).session(&mut session).await?;
    if changed.matched_count == 1 {
        state.db.collection::<Document>(access::OUTBOX).update_one(
            doc!{"kind":"owner_job","job_id":id},
            doc!{"$setOnInsert":{"_id":Uuid::new_v4().to_string(),"kind":"owner_job","job_id":id,"node_id":node_id,"user_id":owner,"runtime_id":&job.runtime_id,"execution_deadline":bson::DateTime::from_chrono(job.expires_at),"pending":true,"claim_until":bson::DateTime::from_millis(0),"created_at":bson::DateTime::now()}}
        ).upsert(true).session(&mut session).await?;
        if let Ok((_, _, _, _, _, _, lease_id)) = external_binding(&job)
            && let Some(lease) = state
                .db
                .collection::<ExternalLease>(access::LEASES)
                .find_one(doc! {"_id":lease_id,"kind":"external"})
                .session(&mut session)
                .await?
        {
            enqueue_lease_in_session(
                &state.db,
                &lease.external_authority,
                "owner_cancelled",
                &mut session,
            )
            .await?;
            state
                .db
                .collection::<Document>(access::LEASES)
                .delete_one(doc! {"_id":lease_id})
                .session(&mut session)
                .await?;
        }
    }
    session.commit_transaction().await?;
    let stopped = if node
        .machine
        .as_ref()
        .is_some_and(|p| p.runtime_id == job.runtime_id)
    {
        send_owner_job(state, &node, &job, Operation::JobCancel, json!({}))
            .await
            .is_ok_and(|v| v["cancel_requested"] == true)
    } else {
        false
    };
    if stopped {
        state.db.collection::<Document>(access::OUTBOX).update_one(doc!{"kind":"owner_job","job_id":id,"pending":true},doc!{"$set":{"pending":false,"completed_at":bson::DateTime::now(),"acknowledged_at":bson::DateTime::now(),"completion_evidence":"node_acknowledged"}}).await?;
    }
    Ok(
        json!({"job_id":id,"status":"cancelled","cancellation_acknowledged":stopped,"stop_acknowledged":false}),
    )
}

pub struct ExpectedConfirmation<'a> {
    pub grant_revision: i64,
    pub runtime_generation: u64,
    pub parameters_digest: &'a str,
}

pub async fn owner_decide_bound(
    db: &Database,
    owner: &str,
    node_id: &str,
    id: &str,
    approve: bool,
    expected: ExpectedConfirmation<'_>,
) -> AppResult<Value> {
    let runtime_generation = i64::try_from(expected.runtime_generation)
        .map_err(|_| invalid("Invalid runtime generation"))?;
    let row=db.collection::<ExternalConfirmation>(execution::CONFIRMATIONS).find_one(doc!{"_id":id,"user_id":owner,"node_id":node_id,"grant_revision":expected.grant_revision,"runtime_generation":runtime_generation,"parameters_digest":expected.parameters_digest}).await?.ok_or(AppError::MachineConfirmationDeclined)?;
    let result = owner_decide(db, owner, &row.id, approve).await?;
    Ok(json!({"confirmation":{
        "id":result["id"],"node_id":result["node_id"],"subject":result["principal"],"grant_id":result["grant_id"],"grant_revision":result["grant_revision"],"assignment_revision":result["assignment_revision"],"runtime_generation":result["runtime_generation"],"operation":result["operation"],"parameters_digest":result["parameters_digest"],"safe_parameters":result["safe_parameters"],"review_parameters_available":result["review_parameters_available"],"status":result["state"],"created_at":result["created_at"],"expires_at":result["expires_at"],"approved_at":if approve{result["decided_at"].clone()}else{Value::Null},"approved_until":if approve{result["expires_at"].clone()}else{Value::Null}
    }}))
}

#[cfg(test)]
#[allow(dead_code)]
fn verify_send_contracts(
    state: &AppState,
    source: &VerifiedExternalSource,
    node: &Node,
    job: &MachineJob,
    session: &mut ClientSession,
) {
    fn send<T: Send>(_: T) {}
    send(visible(&state.db, source));
    send(gateway_auth(&state.db, job));
    send(admit_request(
        state,
        source,
        node,
        Operation::Exec,
        &json!({}),
        None,
    ));
    send(renew_job_with_source(state, source, job));
    send(history(&state.db, source, &node.id, &json!({})));
    send(owner_output(state, &node.user_id, &node.id, &job.id, 0, 0));
    send(grants::admit_in_session(
        &state.db, source, &node.id, None, session,
    ));
    send(super::machine_gateway_service::external_services(
        &state.db,
        &node.user_id,
        &[],
    ));
}
