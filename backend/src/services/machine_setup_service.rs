//! Owner-reviewed setup intents and device-style machine pairing.
use crate::{
    errors::{AppError, AppResult},
    models::{
        machine_setup::{COLLECTION_NAME, Choices, MachineSetup},
        nyxbot_channel::{NyxbotWatch, WATCHES_COLLECTION_NAME},
    },
};
use chrono::{Duration, Utc};
use hmac::{Hmac, Mac};
use mongodb::{
    Database,
    bson::{self, doc},
    options::ReturnDocument,
};
use rand::Rng;
use sha2::Sha256;
use zeroize::Zeroizing;

pub const TTL_SECONDS: i64 = 900;

pub fn digest(key: &[u8], domain: &str, input: &str) -> String {
    let mut hmac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts any key length");
    hmac.update(b"nyxid.machine.pair.v1\0");
    hmac.update(domain.as_bytes());
    hmac.update(&[0]);
    hmac.update(input.as_bytes());
    hex::encode(hmac.finalize().into_bytes())
}

pub fn normalize_code(code: &str) -> AppResult<String> {
    if code.len() > 16 {
        return Err(AppError::AuthDeviceUserCodeInvalid);
    }
    let normalized: String = code
        .chars()
        .filter(|c| *c != '-' && *c != ' ')
        .flat_map(char::to_uppercase)
        .collect();
    if normalized.len() != 8
        || !normalized
            .bytes()
            .all(|c| b"23456789ABCDEFGHJKMNPQRSTVWXYZ".contains(&c))
    {
        return Err(AppError::AuthDeviceUserCodeInvalid);
    }
    Ok(normalized)
}

pub async fn validate_choices(
    db: &Database,
    owner: &str,
    mut choices: Choices,
) -> AppResult<Choices> {
    if choices.name.is_empty()
        || choices.name.len() > 64
        || !choices
            .name
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
    {
        return Err(AppError::ValidationError(
            "Machine name must contain 1–64 lowercase letters, numbers or hyphens".into(),
        ));
    }
    if !matches!(choices.location.as_str(), "this_computer" | "vm" | "docker") {
        return Err(AppError::ValidationError(
            "Choose this_computer, vm or docker".into(),
        ));
    }
    choices.capabilities.sort();
    choices.capabilities.dedup();
    if choices.capabilities.is_empty()
        || choices
            .capabilities
            .iter()
            .any(|c| !matches!(c.as_str(), "shell" | "files" | "computer"))
    {
        return Err(AppError::ValidationError(
            "Choose shell, files or computer capabilities".into(),
        ));
    }
    if let Some(selected) = &choices.owner_id
        && !super::org_service::resolve_owner_access(db, owner, selected)
            .await?
            .can_write()
    {
        return Err(AppError::Forbidden(
            "Machine setup requires owner or organization admin access".into(),
        ));
    }
    if let Some(agent) = choices.grant_to.as_deref() {
        choices.grant_to = Some(
            super::assistant_team_service::live_specialist(db, owner, agent)
                .await?
                .id,
        );
    }
    Ok(choices)
}

pub async fn create_link(
    db: &Database,
    owner: &str,
    conversation: Option<&str>,
    choices: Choices,
) -> AppResult<MachineSetup> {
    let choices = validate_choices(db, owner, choices).await?;
    let now = Utc::now();
    let row = MachineSetup {
        id: uuid::Uuid::new_v4().to_string(),
        user_id: owner.into(),
        choices,
        status: "review".into(),
        code_hmac: None,
        device_hmac: None,
        hostname: None,
        os: None,
        ip: None,
        conversation_id: conversation.map(str::to_owned),
        last_poll_at: None,
        created_at: now,
        expires_at: now + Duration::seconds(TTL_SECONDS),
        purge_at: now + Duration::days(1),
    };
    db.collection::<MachineSetup>(COLLECTION_NAME)
        .insert_one(&row)
        .await?;
    if let Some(conversation) = conversation {
        watch(db, owner, conversation, &row.id, row.expires_at).await?;
    }
    Ok(row)
}

pub struct Pairing {
    pub code: String,
    pub device: Zeroizing<String>,
}
impl std::fmt::Debug for Pairing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Pairing { [REDACTED] }")
    }
}

pub async fn initiate(
    db: &Database,
    key: &[u8],
    hostname: &str,
    os: &str,
    ip: &str,
    capabilities: Vec<String>,
) -> AppResult<Pairing> {
    if hostname.is_empty()
        || hostname.len() > 128
        || hostname.chars().any(char::is_control)
        || !matches!(os, "linux" | "macos")
    {
        return Err(AppError::ValidationError(
            "Invalid machine hostname or OS".into(),
        ));
    }
    let name: String = hostname
        .to_ascii_lowercase()
        .bytes()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c as char
            } else {
                '-'
            }
        })
        .take(48)
        .collect();
    let choices = validate_choices(
        db,
        "",
        Choices {
            owner_id: None,
            name,
            location: "vm".into(),
            capabilities,
            grant_to: None,
        },
    )
    .await?;
    for _ in 0..5 {
        let code: String = (0..8)
            .map(|_| {
                let alphabet = b"23456789ABCDEFGHJKMNPQRSTVWXYZ";
                alphabet[rand::thread_rng().gen_range(0..alphabet.len())] as char
            })
            .collect();
        let device = Zeroizing::new(hex::encode(rand::random::<[u8; 32]>()));
        let now = Utc::now();
        let row = MachineSetup {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: String::new(),
            choices: choices.clone(),
            status: "pending".into(),
            code_hmac: Some(digest(key, "code", &code)),
            device_hmac: Some(digest(key, "device", &device)),
            hostname: Some(hostname.into()),
            os: Some(os.into()),
            ip: Some(ip.into()),
            conversation_id: None,
            last_poll_at: None,
            created_at: now,
            expires_at: now + Duration::seconds(TTL_SECONDS),
            purge_at: now + Duration::days(1),
        };
        match db
            .collection::<MachineSetup>(COLLECTION_NAME)
            .insert_one(&row)
            .await
        {
            Ok(_) => {
                return Ok(Pairing {
                    code: format!("{}-{}", &code[..4], &code[4..]),
                    device,
                });
            }
            Err(error) if error.to_string().contains("E11000") => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(AppError::AuthDeviceCodeRateLimited)
}

pub async fn by_code(db: &Database, key: &[u8], code: &str) -> AppResult<MachineSetup> {
    let hash = digest(key, "code", &normalize_code(code)?);
    let row = db
        .collection::<MachineSetup>(COLLECTION_NAME)
        .find_one(doc! {"code_hmac":hash})
        .await?
        .ok_or(AppError::AuthDeviceUserCodeInvalid)?;
    if row.expires_at <= Utc::now() {
        return Err(AppError::AuthDeviceCodeExpired);
    }
    Ok(row)
}

pub async fn get(db: &Database, owner: &str, id: &str) -> AppResult<MachineSetup> {
    db.collection::<MachineSetup>(COLLECTION_NAME)
        .find_one(doc! {"_id":id,"user_id":owner})
        .await?
        .ok_or_else(|| AppError::NotFound("Machine setup not found".into()))
}

/// Page-only setups have no waiting NyxBot to apply the reviewed grant. Commit
/// the grant and completion together so a reconnect cannot restore a later
/// revoked grant. Chat-led setups leave verification and granting to NyxBot.
pub async fn complete_page_setup(db: &Database, id: &str) -> AppResult<()> {
    use super::{api_key_mutation_service as transactions, assistant_team_service as team};
    let Some(row) = db
        .collection::<MachineSetup>(COLLECTION_NAME)
        .find_one(doc! {"_id":id,"conversation_id":null,"status":"waiting"})
        .await?
    else {
        return Ok(());
    };
    let Some(node) = super::node_service::get_node_by_id(db, id).await? else {
        return Ok(());
    };
    if node.user_id != row.choices.owner_id.as_deref().unwrap_or(&row.user_id)
        || node.status != crate::models::node::NodeStatus::Online
        || node
            .machine
            .as_ref()
            .is_none_or(|m| !m.enabled() || (m.computer && !m.computer_ready))
        || !super::org_service::resolve_owner_access(db, &row.user_id, &node.user_id)
            .await?
            .can_write()
    {
        return Ok(());
    }
    let mut session = db.client().start_session().await?;
    let db = db.clone();
    let id = id.to_owned();
    session
        .start_transaction()
        .and_run2(async move |session| {
            let result: AppResult<()> = async {
                let changed = db
                    .collection::<MachineSetup>(COLLECTION_NAME)
                    .update_one(
                        doc! {"_id":&id,"status":"waiting","conversation_id":null},
                        doc! {"$set":{"status":"connected"}},
                    )
                    .session(&mut *session)
                    .await?;
                if changed.modified_count == 1
                    && let Some(agent) = row.choices.grant_to.as_deref()
                {
                    team::apply_grants_in_session(
                        &db,
                        &row.user_id,
                        agent,
                        &team::GrantChange::Machine {
                            base: Box::new(team::GrantChange::Add(Default::default())),
                            machines: Some(vec![id.to_owned()]),
                            logins: None,
                            mode: team::MachineGrantMode::Add,
                        },
                        session,
                    )
                    .await?;
                }
                Ok(())
            }
            .await;
            transactions::transaction_result(result)
        })
        .await
        .map_err(transactions::map_transaction_error)
}

pub async fn decide(
    db: &Database,
    owner: &str,
    id: &str,
    approve: bool,
    conversation: Option<&str>,
) -> AppResult<MachineSetup> {
    let row = db.collection::<MachineSetup>(COLLECTION_NAME).find_one_and_update(
        doc! {"_id":id,"status":"pending","expires_at":{"$gt":bson::DateTime::now()}},
        doc! {"$set":{"user_id":owner,"status":if approve {"approved"} else {"declined"},"conversation_id":conversation}},
    ).return_document(ReturnDocument::After).await?.ok_or_else(|| AppError::Conflict("Pairing was already decided or expired".into()))?;
    if let Some(conversation) = conversation {
        watch(db, owner, conversation, &row.id, row.expires_at).await?;
    }
    Ok(row)
}

pub async fn watch(
    db: &Database,
    owner: &str,
    conversation: &str,
    id: &str,
    expires: chrono::DateTime<Utc>,
) -> AppResult<()> {
    let now = Utc::now();
    db.collection::<NyxbotWatch>(WATCHES_COLLECTION_NAME).update_one(
        doc! {"kind":"machine_setup","connect_link_id":id,"user_id":owner},
        doc! {"$setOnInsert":{"_id":uuid::Uuid::new_v4().to_string(),"user_id":owner,"kind":"machine_setup","connect_link_id":id,"conversation_id":conversation,"status":"pending","created_at":bson::DateTime::from_chrono(now),"expires_at":bson::DateTime::from_chrono(expires+Duration::hours(1))}},
    ).upsert(true).await?;
    Ok(())
}

/// Claim the delivery before minting. Losing a response never delivers a second credential.
pub async fn mint(
    db: &Database,
    owner: &str,
    id: &str,
    choices: Option<Choices>,
    max_nodes: u32,
    expected_status: &str,
) -> AppResult<Zeroizing<String>> {
    let choices = match choices {
        Some(c) => Some(validate_choices(db, owner, c).await?),
        None => None,
    };
    let mut set = doc! {"status":"issuing"};
    if let Some(choices) = choices {
        set.insert(
            "choices",
            bson::to_bson(&choices)
                .map_err(|_| AppError::Internal("Could not encode machine setup choices".into()))?,
        );
    }
    let row = db.collection::<MachineSetup>(COLLECTION_NAME).find_one_and_update(
        doc! {"_id":id,"user_id":owner,"status":expected_status,"expires_at":{"$gt":bson::DateTime::now()}},doc! {"$set":set},
    ).return_document(ReturnDocument::After).await?.ok_or_else(|| AppError::Conflict("Setup expired or its command was already issued; create a new setup".into()))?;
    let result = super::node_service::create_registration_token_with_id(
        db,
        row.choices.owner_id.as_deref().unwrap_or(owner),
        &row.choices.name,
        max_nodes,
        (row.expires_at - Utc::now()).num_seconds().max(1),
        id,
    )
    .await;
    let status = if result.is_ok() { "waiting" } else { "failed" };
    db.collection::<MachineSetup>(COLLECTION_NAME)
        .update_one(
            doc! {"_id":id,"status":"issuing"},
            doc! {"$set":{"status":status}},
        )
        .await?;
    Ok(Zeroizing::new(result?.1))
}

pub async fn poll(
    db: &Database,
    key: &[u8],
    device: &str,
    max_nodes: u32,
) -> AppResult<Option<Zeroizing<String>>> {
    if device.len() != 64 {
        return Err(AppError::AuthDeviceCodeNotFound);
    }
    let hash = digest(key, "device", device);
    let now = Utc::now();
    let row = db
        .collection::<MachineSetup>(COLLECTION_NAME)
        .find_one(doc! {"device_hmac":&hash})
        .await?
        .ok_or(AppError::AuthDeviceCodeNotFound)?;
    if row.expires_at <= now {
        return Err(AppError::AuthDeviceCodeExpired);
    }
    if row
        .last_poll_at
        .is_some_and(|last| now - last < Duration::seconds(2))
    {
        return Err(AppError::AuthDeviceCodeSlowDown);
    }
    let claimed = db.collection::<MachineSetup>(COLLECTION_NAME).update_one(
        doc! {"_id":&row.id,"$or":[{"last_poll_at":null},{"last_poll_at":{"$lte":bson::DateTime::from_chrono(now-Duration::seconds(2))}}]},
        doc! {"$set":{"last_poll_at":bson::DateTime::from_chrono(now)}},
    ).await?;
    if claimed.modified_count == 0 {
        return Err(AppError::AuthDeviceCodeSlowDown);
    }
    match row.status.as_str() {
        "pending" => Ok(None),
        "approved" => Ok(Some(
            mint(db, &row.user_id, &row.id, None, max_nodes, "approved").await?,
        )),
        "declined" => Err(AppError::AuthDeviceCodeDenied),
        _ => Err(AppError::AuthDeviceCodeAlreadyDelivered),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn codes_are_normalized_and_hashes_domain_separated() {
        assert_eq!(normalize_code("abcd-2345").unwrap(), "ABCD2345");
        assert!(normalize_code("credential").is_err());
        assert_ne!(
            digest(b"test", "code", "ABCD2345"),
            digest(b"test", "device", "ABCD2345")
        );
        assert!(!digest(b"test", "code", "ABCD2345").contains("ABCD2345"));
    }
}
