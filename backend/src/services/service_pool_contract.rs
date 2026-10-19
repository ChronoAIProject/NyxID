//! One compatibility rule for saved configuration and live execution metadata.
use crate::errors::{AppError, AppResult};
use crate::models::{
    downstream_service::{DownstreamService, InferenceWireProtocol},
    service_pool::{PoolMemberContract, PoolStrategy, ServicePoolMember},
    user_service::UserService,
};
use mongodb::bson::doc;

#[derive(Clone)]
pub struct ContractMember {
    pub catalog_id: Option<String>,
    pub protocol: Option<InferenceWireProtocol>,
    pub declared: bool,
}

pub fn declaration_required(member: &ContractMember, peers: &[ContractMember]) -> bool {
    member.catalog_id.is_none()
        || peers
            .iter()
            .any(|other| other.catalog_id != member.catalog_id)
}

pub fn validate(contract: PoolMemberContract, members: &[ContractMember]) -> AppResult<()> {
    if contract == PoolMemberContract::AiChat {
        if members.iter().any(|member| member.protocol.is_none()) {
            return Err(AppError::ServicePoolMemberInvalid(
                "AI chat members require authoritative catalog inference metadata".into(),
            ));
        }
        return Ok(());
    }
    let protocol = members.iter().find_map(|member| member.protocol);
    if members
        .iter()
        .filter_map(|member| member.protocol)
        .any(|p| Some(p) != protocol)
    {
        return Err(AppError::ServicePoolMemberInvalid(
            "Same API members have incompatible inference protocols".into(),
        ));
    }
    if members
        .iter()
        .any(|member| !member.declared && declaration_required(member, members))
    {
        return Err(AppError::ServicePoolMemberInvalid("Custom or different catalog APIs require an explicit same_api_compatible declaration on every affected member".into()));
    }
    Ok(())
}

pub async fn validate_configuration(
    db: &mongodb::Database,
    owner: &str,
    strategy: PoolStrategy,
    contract: PoolMemberContract,
    members: &[ServicePoolMember],
) -> AppResult<()> {
    if strategy != PoolStrategy::Priority {
        return Ok(());
    }
    validate(contract, &load_metadata(db, owner, members, false).await?)
}

pub async fn load_metadata(
    db: &mongodb::Database,
    owner: &str,
    members: &[ServicePoolMember],
    allow_missing: bool,
) -> AppResult<Vec<ContractMember>> {
    let mut metadata = Vec::new();
    for member in members {
        let service =
            crate::services::service_history::collection::<UserService>(db, "user_services")
                .find_one(doc! {"_id":&member.user_service_id,"user_id":owner})
                .await?;
        let Some(service) = service else {
            if allow_missing {
                continue;
            }
            return Err(AppError::ServicePoolMemberInvalid(
                "Member is unavailable".into(),
            ));
        };
        let catalog = if let Some(id) = &service.catalog_service_id {
            db.collection::<DownstreamService>("downstream_services")
                .find_one(doc! {"_id":id})
                .await?
        } else {
            None
        };
        metadata.push(ContractMember {
            catalog_id: catalog.as_ref().map(|catalog| catalog.id.clone()),
            protocol: catalog
                .and_then(|catalog| catalog.inference)
                .map(|inference| inference.wire_protocol),
            declared: member.same_api_compatible,
        });
    }
    Ok(metadata)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn service_pool_contract_requires_declarations_but_never_overrides_known_mismatch() {
        let mut members = vec![
            ContractMember {
                catalog_id: Some("a".into()),
                protocol: Some(InferenceWireProtocol::OpenaiCompletions),
                declared: false,
            },
            ContractMember {
                catalog_id: Some("b".into()),
                protocol: Some(InferenceWireProtocol::OpenaiCompletions),
                declared: false,
            },
        ];
        assert!(validate(PoolMemberContract::SameApi, &members).is_err());
        for member in &mut members {
            member.declared = true;
        }
        assert!(validate(PoolMemberContract::SameApi, &members).is_ok());
        members[1].protocol = Some(InferenceWireProtocol::AnthropicMessages);
        assert!(validate(PoolMemberContract::SameApi, &members).is_err());
        assert!(validate(PoolMemberContract::AiChat, &members).is_ok());
        members[1].protocol = None;
        members[1].catalog_id = None;
        assert!(validate(PoolMemberContract::AiChat, &members).is_err());
        assert!(declaration_required(&members[1], &members));
    }
}
