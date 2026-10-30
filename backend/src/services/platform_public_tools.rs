//! Closed public help tools. No ordinary inventory, credentials or tool router.
use super::{
    api_key_mutation_service as tx, platform_manager_contract::*,
    platform_public_budget_service as budget, platform_public_policy as policy,
};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        api_key::{ApiKey, ApiKeyPurpose},
        platform_public_session::{COLLECTION_NAME as SESSIONS, PlatformPublicSession},
    },
};
use mongodb::{Database, bson::doc};
use serde::Deserialize;
use serde_json::{Value, json};
use zeroize::Zeroizing;

pub(crate) const INSTRUCTIONS: &str = "You are the NyxID public guide. Answer only using the approved public FAQ tools. Explain NyxID and its use cases, and offer public_signup_link when the visitor wants to review or create their own account. You have no personal account, memory, connected services, private documents or general tools. Never request passwords, keys or login codes. A signup link does not confer consent; personal linking requires website review and a final original confirmation command in the same private chat.";
// Leave room for tool definitions, instructions and JSON escaping in the
// public model's separately enforced request budget. Articles are lossless pages.
const MAX_RESULT_BYTES: usize = 2048;
const ARTICLE_PAGE_BYTES: usize = 1024;
pub(crate) const COMPACT_INSTRUCTIONS: &str = "Summarize this public FAQ conversation as plain text, preserving approved FAQ facts and unresolved questions. Do not call tools or add private account information.";
pub(crate) const COMPACT_PROMPT: &str = "You are performing a CONTEXT CHECKPOINT COMPACTION. Create a handoff summary for another LLM that will resume the task.\n\nInclude:\n- Current progress and key decisions made\n- Important context, constraints, or user preferences\n- What remains to be done (clear next steps)\n- Any critical data, examples, or references needed to continue\n\nBe concise, structured, and focused on helping the next LLM seamlessly continue the work.\n";
const WRAPPER: &str = "nyx__call_tool";

fn denied() -> AppError {
    AppError::Forbidden("Public tool or arguments are not allowed".into())
}

/// Not an AuthUser or an ordinary MCP context. Fields cannot be supplied by RPC.
pub(crate) struct PublicMcpAuthority {
    context: PublicExecutionContext,
    key_id: String,
    _proof: policy::VerifiedPublicContract,
}

impl PublicMcpAuthority {
    pub(crate) fn context(&self) -> &PublicExecutionContext {
        &self.context
    }
    pub(crate) fn metadata(&self) -> Value {
        json!({"nyxid/public":{"contract":"nyxid-public-execution.v1","purpose":"public_nyxbot",
            "profile":self.context.nyxagent_profile,"policy_digest":self.context.policy_digest,"proof_record_id":self.context.proof_record_id,
            "public_session_id":self.context.public_session_id,"turn_id":self.context.turn.turn_id,
            "receipt_id":self.context.ingress.ingress.receipt_id,"claim_id":self.context.ingress.ingress.claim_id,
            "expires_at":self.context.expires_at.to_rfc3339()}})
    }
    pub(crate) fn key_id(&self) -> &str {
        &self.key_id
    }
    pub(crate) fn validate_callback_headers(&self, headers: &http::HeaderMap) -> AppResult<()> {
        for (name, expected) in [
            (
                "x-nyx-public-session-id",
                self.context.public_session_id.as_str(),
            ),
            ("x-nyx-public-turn-id", self.context.turn.turn_id.as_str()),
            (
                "x-nyx-public-receipt-id",
                self.context.ingress.ingress.receipt_id.as_str(),
            ),
            (
                "x-nyx-public-claim-id",
                self.context.ingress.ingress.claim_id.as_str(),
            ),
        ] {
            if headers.get_all(name).iter().count() != 1
                || headers.get(name).and_then(|v| v.to_str().ok()) != Some(expected)
            {
                return Err(denied());
            }
        }
        Ok(())
    }
}

pub(crate) async fn fence_callback_in_session(
    db: &Database,
    session: &mut mongodb::ClientSession,
    context: &PublicExecutionContext,
    key: &str,
) -> AppResult<()> {
    policy::verified_context_in_session(db, session, context).await?;
    if db.collection::<mongodb::bson::Document>(crate::models::platform_public_turn::COLLECTION_NAME)
        .update_one(doc! {"_id":&context.turn.turn_id,"public_session_id":&context.public_session_id,
            "receipt_id":&context.ingress.ingress.receipt_id,"claim_id":&context.ingress.ingress.claim_id,
            "credential_api_key_id":key,"policy_digest":&context.policy_digest,"outer_dispatch_admitted":true,
            "reply_id":mongodb::bson::Bson::Null},doc! {"$inc":{"callback_fence":1_i64}})
        .session(session).await?.matched_count!=1 {return Err(denied());}
    Ok(())
}

pub(crate) async fn authenticate(
    db: &Database,
    key: &ApiKey,
    child: Option<&str>,
) -> AppResult<PublicMcpAuthority> {
    if key.purpose != ApiKeyPurpose::PublicNyxbot || child.is_some() || !key.is_active {
        return Err(denied());
    }
    let row = db
        .collection::<PlatformPublicSession>(SESSIONS)
        .find_one(doc! {"credential_api_key_id":&key.id,"revoked_at":mongodb::bson::Bson::Null})
        .await?
        .ok_or_else(denied)?;
    let turn = row.active_turn.as_ref().ok_or_else(denied)?;
    let retained = db
        .collection::<crate::models::platform_public_turn::PlatformPublicTurn>(
            crate::models::platform_public_turn::COLLECTION_NAME,
        )
        .find_one(
            doc! {"_id":&turn.turn_id,"public_session_id":&row.id,"credential_api_key_id":&key.id},
        )
        .await?
        .ok_or_else(denied)?;
    let manager = ManagerAuthority {
        profile_id: row.profile_id.clone(),
        manager_bot_id: row.manager_bot_id.clone(),
        entry: row.entry.clone(),
        manager_authority_epoch: row.manager_authority_epoch,
    };
    let (_, evidence) = policy::verified_readiness(
        db,
        &manager,
        row.public_policy_revision,
        Some(&row.policy_digest),
        Some(&retained.proof_record_id),
    )
    .await?;
    let context = PublicExecutionContext {
        proof_record_id: evidence.proof_record_id.clone(),
        ingress: PublicIngress {
            ingress: IngressClaim {
                receipt_id: turn.receipt_id.clone(),
                claim_id: turn.claim_id.clone(),
            },
            manager: ManagerAuthority {
                profile_id: row.profile_id,
                manager_bot_id: row.manager_bot_id,
                entry: row.entry.clone(),
                manager_authority_epoch: row.manager_authority_epoch,
            },
            sender: row.sender.clone(),
            public_policy_revision: row.public_policy_revision,
        },
        public_session_id: row.id,
        turn: TurnRef {
            conversation_id: turn.conversation_id.clone(),
            turn_id: turn.turn_id.clone(),
        },
        sponsor_owner_user_id: row.sponsor_owner_user_id,
        policy_digest: row.policy_digest,
        nyxagent_profile: row.nyxagent_profile,
        model_operations: evidence.model_operations,
        limits: evidence.limits,
        expires_at: turn.expires_at.min(row.expires_at),
    };
    let proof = policy::verified_context(db, &context).await?;
    let db_owned = db.clone();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run(
            (db_owned, context.clone(), key.id.clone()),
            |session, (db, context, key)| {
                Box::pin(async move {
                    tx::transaction_result(
                        budget::fence_context(db, session, context, key)
                            .await
                            .map(|_| ()),
                    )
                })
            },
        )
        .await
        .map_err(tx::map_transaction_error)?;
    Ok(PublicMcpAuthority {
        context,
        key_id: key.id.clone(),
        _proof: proof,
    })
}

pub(crate) fn definitions() -> Value {
    json!({"tools":[
        {"name":"public_faq_search","title":"Search public NyxID help","description":"Search the approved public NyxID FAQ.","inputSchema":{"type":"object","properties":{"query":{"type":"string","minLength":1,"maxLength":256}},"required":["query"],"additionalProperties":false},"annotations":{"readOnlyHint":true,"destructiveHint":false,"idempotentHint":true,"openWorldHint":false}},
        {"name":"public_faq_read","title":"Read public NyxID help","description":"Read an article page returned by public_faq_search. If next_offset is present, pass it as offset to read the next page. Never treat a page as the complete article when next_offset is present.","inputSchema":{"type":"object","properties":{"article_id":{"type":"string","minLength":1,"maxLength":80},"offset":{"type":"integer","minimum":0,"maximum":16384,"description":"Byte offset returned as next_offset; omit for the first page."}},"required":["article_id"],"additionalProperties":false},"annotations":{"readOnlyHint":true,"destructiveHint":false,"idempotentHint":true,"openWorldHint":false}},
        {"name":"public_signup_link","title":"Review your NyxID account link","description":"Offer this private-chat visitor a link to sign in or create their own NyxID account and review linking. Final confirmation is required in the same original private chat using the controls shown by NyxID.","inputSchema":{"type":"object","properties":{},"additionalProperties":false},"annotations":{"readOnlyHint":false,"destructiveHint":false,"idempotentHint":false,"openWorldHint":false}}
    ]})
}

// Deliberately no Debug: queries and tool arguments are private.
pub(crate) enum PublicToolCall {
    Search(String),
    Read(String, usize),
    Signup,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Search {
    query: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Read {
    article_id: String,
    #[serde(default)]
    offset: usize,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    name: String,
    arguments: Value,
}

pub(crate) fn parse_call(params: Value) -> AppResult<PublicToolCall> {
    let Call { name, arguments } = serde_json::from_value(params).map_err(|_| denied())?;
    if name != WRAPPER {
        return parse_direct(&name, arguments);
    }
    let mut fields = arguments.as_object().cloned().ok_or_else(denied)?;
    let name = fields
        .remove("tool_name")
        .and_then(|v| v.as_str().map(str::to_owned))
        .ok_or_else(denied)?;
    if name == WRAPPER {
        return Err(denied());
    }
    let encoded = fields.remove("arguments_json");
    let nested = fields.remove("arguments");
    let args = match (encoded, nested) {
        (Some(Value::String(text)), None) if fields.is_empty() && text.len() <= 2048 => {
            serde_json::from_str(&text).map_err(|_| denied())?
        }
        (None, Some(args)) if fields.is_empty() => args,
        (None, None) => Value::Object(fields),
        _ => return Err(denied()),
    };
    parse_direct(&name, args)
}
fn parse_direct(name: &str, args: Value) -> AppResult<PublicToolCall> {
    if !args.is_object() {
        return Err(denied());
    }
    match name {
        "public_faq_search" => {
            let input: Search = serde_json::from_value(args).map_err(|_| denied())?;
            if input.query.trim().is_empty()
                || input.query.len() > 256
                || input.query.chars().any(char::is_control)
            {
                return Err(denied());
            }
            Ok(PublicToolCall::Search(input.query))
        }
        "public_faq_read" => {
            let input: Read = serde_json::from_value(args).map_err(|_| denied())?;
            if !super::platform_help_service::valid_id(&input.article_id) || input.offset > 16384 {
                return Err(denied());
            }
            Ok(PublicToolCall::Read(input.article_id, input.offset))
        }
        "public_signup_link" => {
            let _: Empty = serde_json::from_value(args).map_err(|_| denied())?;
            Ok(PublicToolCall::Signup)
        }
        _ => Err(denied()),
    }
}

#[cfg(test)]
fn corpus() -> AppResult<Vec<crate::models::platform_help::HelpArticle>> {
    Ok(super::platform_help_service::seed_content()?.articles)
}
#[cfg(test)]
fn faq(call: &PublicToolCall) -> AppResult<Value> {
    faq_from(call, corpus()?)
}
fn faq_from(
    call: &PublicToolCall,
    articles: Vec<crate::models::platform_help::HelpArticle>,
) -> AppResult<Value> {
    match call {
        PublicToolCall::Read(id, offset) => {
            let article = articles
                .into_iter()
                .find(|a| a.id == *id)
                .ok_or_else(denied)?;
            let remainder = article.body.get(*offset..).ok_or_else(denied)?;
            // Budget the serialized string as it will appear inside a model
            // request, including the second JSON encoding of the tool result.
            let mut end = *offset;
            let mut bytes = 0;
            for ch in remainder.chars() {
                let encoded = serde_json::to_string(&ch.to_string()).map_err(|_| denied())?;
                let cost = serde_json::to_string(&encoded[1..encoded.len() - 1])
                    .map_err(|_| denied())?
                    .len()
                    - 2;
                if bytes + cost > ARTICLE_PAGE_BYTES {
                    break;
                }
                bytes += cost;
                end += ch.len_utf8();
            }
            let mut page = json!({"article_id":article.id,"title":article.title,
                "text":&article.body[*offset..end],"offset":offset});
            if end < article.body.len() {
                page["next_offset"] = json!(end);
            }
            Ok(page)
        }
        PublicToolCall::Search(query) => {
            let words: Vec<_> = query
                .split(|c: char| !c.is_alphanumeric())
                .filter(|s| !s.is_empty())
                .map(str::to_lowercase)
                .collect();
            let mut matches: Vec<_> = articles
                .into_iter()
                .filter_map(|a| {
                    let haystack =
                        format!("{} {} {}", a.title, a.body, a.tags.join(" ")).to_lowercase();
                    let score = words
                        .iter()
                        .filter(|w| haystack.contains(w.as_str()))
                        .count();
                    (score > 0).then_some((score, a))
                })
                .collect();
            matches.sort_by(|(sa, a), (sb, b)| sb.cmp(sa).then(a.id.cmp(&b.id)));
            Ok(
                json!({"articles":matches.into_iter().take(5).map(|(_,a)|json!({"article_id":a.id,"title":a.title})).collect::<Vec<_>>()}),
            )
        }
        PublicToolCall::Signup => Err(denied()),
    }
}

/// Mint commits before the separate ephemeral provider effect. Only delivery
/// status reaches the model; the bearer never enters its transcript or reply.
pub(crate) async fn execute(
    state: &AppState,
    authority: &PublicMcpAuthority,
    call: PublicToolCall,
) -> AppResult<Zeroizing<String>> {
    policy::verified_context(&state.db, &authority.context).await?;
    let db = state.db.clone();
    let mut session = db.client().start_session().await?;
    let context = authority.context.clone();
    let key = authority.key_id.clone();
    let hmac_key = state.auth_device_hmac_key.clone();
    let value = session
        .start_transaction()
        .and_run(
            (db, context, key, call, hmac_key),
            |session, (db, context, key, call, hmac_key)| {
                Box::pin(async move {
                    let result: AppResult<_> = async {
                        fence_callback_in_session(db, session, context, key).await?;
                        budget::admit_tool_call_in_session(db, session, context, key).await?;
                        match call {
                            PublicToolCall::Signup => {
                                let issued =
                                    super::platform_link_intent_service::begin_public_in_session(
                                        db,
                                        session,
                                        hmac_key.as_slice(),
                                        context,
                                    )
                                    .await?;
                                Ok(Ok(issued.token))
                            }
                            _ => {
                                let help = super::platform_help_service::for_turn(
                                    db,
                                    session,
                                    &context.ingress.manager.profile_id,
                                    &context.turn.turn_id,
                                )
                                .await?;
                                Ok(Err(faq_from(
                                    call,
                                    super::platform_faq_service::all_articles(&help.content),
                                )?))
                            }
                        }
                    }
                    .await;
                    tx::transaction_result(result)
                })
            },
        )
        .await
        .map_err(tx::map_transaction_error)?;
    let chat = super::platform_chat_presentation::ChatPresentation::for_entry(
        &authority.context.ingress.manager.entry,
    )?;
    let output = match value {
        Ok(Some(token)) => {
            let status=super::platform_manager_delivery::deliver_public_signup(state,&authority.context,token).await?;
            Zeroizing::new(serde_json::to_string(&signup_result(status,chat)).map_err(|_|denied())?)
        }
        Ok(None) => Zeroizing::new(serde_json::to_string(&json!({
            "status":"already_issued","message":format!("A link was already issued for this request. If it is unavailable, send {} using this private chat's original control surface.",chat.login())
        })).map_err(|_|denied())?),
        Err(value) => Zeroizing::new(serde_json::to_string(&value).map_err(|_|denied())?),
    };
    if output.len() > MAX_RESULT_BYTES {
        return Err(denied());
    }
    Ok(output)
}

fn signup_result(
    status: super::platform_manager_delivery::PublicSignupDeliveryOutcome,
    chat: super::platform_chat_presentation::ChatPresentation,
) -> Value {
    use super::platform_manager_delivery::PublicSignupDeliveryOutcome;
    match status {
        PublicSignupDeliveryOutcome::Sent => {
            json!({"status":"sent","message":"The account review link was sent separately in this private chat. Website review and a final original confirmation are still required."})
        }
        PublicSignupDeliveryOutcome::Unavailable => {
            json!({"status":"unavailable","message":format!("The link could not be delivered. Send {} using this private chat's original control surface.",chat.login())})
        }
        PublicSignupDeliveryOutcome::Uncertain => {
            json!({"status":"uncertain","message":format!("Link delivery could not be confirmed. If no link arrived, send {} using this private chat's original control surface.",chat.login())})
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_callbacks_require_the_exact_turn_claim_even_when_session_key_is_reused() {
        let (_, key, context) = super::super::platform_public_credential_service::tests::fixture();
        let authority = PublicMcpAuthority {
            _proof: policy::VerifiedPublicContract::for_local_tests(context.policy_digest.clone()),
            context,
            key_id: key.id,
        };
        let mut headers = http::HeaderMap::new();
        for (name, value) in [
            (
                "x-nyx-public-session-id",
                authority.context.public_session_id.as_str(),
            ),
            (
                "x-nyx-public-turn-id",
                authority.context.turn.turn_id.as_str(),
            ),
            (
                "x-nyx-public-receipt-id",
                authority.context.ingress.ingress.receipt_id.as_str(),
            ),
            (
                "x-nyx-public-claim-id",
                authority.context.ingress.ingress.claim_id.as_str(),
            ),
        ] {
            headers.insert(name, http::HeaderValue::from_str(value).unwrap());
        }
        assert!(authority.validate_callback_headers(&headers).is_ok());
        let mut stale = headers.clone();
        stale.insert(
            "x-nyx-public-turn-id",
            http::HeaderValue::from_static("old"),
        );
        assert!(authority.validate_callback_headers(&stale).is_err());
        let mut stale = headers.clone();
        stale.append(
            "x-nyx-public-claim-id",
            http::HeaderValue::from_static("other"),
        );
        assert!(authority.validate_callback_headers(&stale).is_err());
        headers.remove("x-nyx-public-session-id");
        assert!(authority.validate_callback_headers(&headers).is_err());
    }
    #[test]
    fn signup_results_are_status_only_and_do_not_claim_consent_or_retry() {
        use super::super::platform_manager_delivery::PublicSignupDeliveryOutcome::*;
        for (state, expected) in [
            (Sent, "sent"),
            (Unavailable, "unavailable"),
            (Uncertain, "uncertain"),
        ] {
            let result = signup_result(
                state,
                super::super::platform_chat_presentation::ChatPresentation::Telegram,
            );
            assert_eq!(result["status"], expected);
            let encoded = result.to_string();
            assert!(!encoded.contains("nyx_pli_"));
            assert!(!encoded.contains("https://"));
            assert!(result.get("token").is_none());
            assert!(result.get("url").is_none());
        }
    }
    #[test]
    fn direct_and_wrapped_tools_share_closed_validation() {
        for args in [
            json!({"tool_name":"public_faq_search","arguments":{"query":"NyxID"}}),
            json!({"tool_name":"public_faq_search","arguments_json":"{\"query\":\"NyxID\"}"}),
            json!({"tool_name":"public_faq_search","query":"NyxID"}),
        ] {
            assert!(matches!(
                parse_call(json!({"name":WRAPPER,"arguments":args})).unwrap(),
                PublicToolCall::Search(_)
            ));
        }
        for name in [
            "nyxid__list_keys",
            "nyx__search_tools",
            "nyx__call_tool",
            "public_faq_search_alias",
            "machine__exec",
            "oracle__run",
            "ssh__exec",
        ] {
            assert!(
                parse_call(json!({"name":WRAPPER,"arguments":{"tool_name":name,"arguments":{}}}))
                    .is_err()
            );
        }
        for args in [
            json!({"tool_name":"public_signup_link","arguments":{},"arguments_json":"{}"}),
            json!({"tool_name":"public_signup_link","arguments":{},"payer":"x"}),
            json!({"tool_name":"public_signup_link","arguments_json":"{}","url":"x"}),
            json!({"tool_name":"public_signup_link","arguments":{"sender_id":5}}),
        ] {
            assert!(parse_call(json!({"name":WRAPPER,"arguments":args})).is_err());
        }
        for arguments in [
            json!(null),
            json!([]),
            json!({"url":"https://example.test"}),
            json!({"owner_id":"other"}),
        ] {
            assert!(
                parse_call(json!({"name":"public_signup_link","arguments":arguments})).is_err()
            );
            assert!(
                parse_call(json!({"name":WRAPPER,"arguments":{"tool_name":"public_signup_link","arguments":arguments}})).is_err()
            );
        }
        for (name, arguments) in [
            ("public_faq_search", json!(["NyxID"])),
            ("public_faq_read", json!(["overview"])),
        ] {
            assert!(parse_call(json!({"name":name,"arguments":arguments})).is_err());
            assert!(
                parse_call(json!({"name":WRAPPER,"arguments":{"tool_name":name,"arguments_json":arguments.to_string()}})).is_err()
            );
        }
        assert!(
            parse_call(json!({"name":"public_faq_search","arguments":{"query":"x".repeat(257)}}))
                .is_err()
        );
        assert!(
            parse_call(json!({"name":"public_faq_read","arguments":{"article_id":"../private"}}))
                .is_err()
        );
        assert!(
            parse_call(json!({"name":"public_signup_link","arguments":{},"approval":true}))
                .is_err()
        );
    }
    #[test]
    fn editable_help_ids_tags_and_escaped_article_bodies_work_in_public_tools() {
        let content = crate::models::platform_help::HelpContent {
            articles: vec![crate::models::platform_help::HelpArticle {
                id: "guide-v2".into(),
                title: "Reviewed guide".into(),
                body: "🦉\"\\\n".repeat(800),
                tags: vec!["bookings".into()],
            }],
            use_cases: vec![],
        };
        super::super::platform_help_service::validate(&content).unwrap();
        let mut offset = 0;
        let mut complete = String::new();
        loop {
            let call = parse_call(json!({"name":"public_faq_read",
                "arguments":{"article_id":"guide-v2","offset":offset}}))
            .unwrap();
            let output = faq_from(&call, content.articles.clone()).unwrap();
            assert_eq!(output["offset"], offset);
            assert!(serde_json::to_vec(&output).unwrap().len() < MAX_RESULT_BYTES);
            let model_content = serde_json::to_string(&output.to_string()).unwrap();
            assert!(model_content.len() < MAX_RESULT_BYTES);
            complete.push_str(output["text"].as_str().unwrap());
            let Some(next) = output["next_offset"].as_u64() else {
                break;
            };
            assert!(next > offset);
            offset = next;
        }
        assert_eq!(complete, content.articles[0].body);
        for offset in [1, 20000] {
            let call = PublicToolCall::Read("guide-v2".into(), offset);
            assert!(faq_from(&call, content.articles.clone()).is_err());
        }
        for offset in [json!(-1), json!(16385), json!(0.5), json!("0")] {
            assert!(
                parse_call(json!({"name":"public_faq_read",
                "arguments":{"article_id":"guide-v2","offset":offset}}))
                .is_err()
            );
        }
        let found = faq_from(&PublicToolCall::Search("bookings".into()), content.articles).unwrap();
        assert_eq!(found["articles"][0]["article_id"], "guide-v2");
    }

    #[test]
    fn advertised_inventory_and_corpus_are_bounded_and_public_only() {
        let definitions = definitions();
        let tools = definitions["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 3);
        assert_eq!(
            tools
                .iter()
                .map(|t| t["name"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["public_faq_search", "public_faq_read", "public_signup_link"]
        );
        let articles = corpus().unwrap();
        assert!(!articles.is_empty() && articles.len() <= 32);
        for a in &articles {
            assert!(a.body.len() < 2048 && a.title.len() < 128);
            assert!(!a.body.contains("http") && !a.body.contains("nyxid_ag_"));
            let output = faq(&PublicToolCall::Read(a.id.clone(), 0)).unwrap();
            assert!(serde_json::to_vec(&output).unwrap().len() < MAX_RESULT_BYTES);
        }
        assert!(faq(&PublicToolCall::Read("private-account".into(), 0)).is_err());
        assert!(
            faq(&PublicToolCall::Search("connected services".into())).unwrap()["articles"]
                .as_array()
                .unwrap()
                .len()
                <= 5
        );
    }
}
