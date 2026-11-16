//! Merchant NyxID-managed phone lines. Lifecycle writes carry a UUID v4 request
//! ID printed before sending; rerun with `--request-id` to replay safely.
use crate::{
    api::ApiClient,
    cli::{AuthArgs, OutputFormat},
    output::print_rows,
};
use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand, ValueEnum};
use serde_json::{Value, json};
use uuid::Uuid;

#[derive(Subcommand)]
pub enum LineCommands {
    /// Show your merchant eligibility, limits and required terms versions
    Eligibility(AuthArgs),
    /// List available NyxID numbers and your claimed lines
    List(AuthArgs),
    /// Show one line with its claim, coverage and renewal quote
    Show(LineArgs),
    /// Price a line: a claim quote by default, or renewal/resume of your claim
    Quote {
        /// Line ID
        line: String,
        /// Quote the next 30-day period of your claim
        #[arg(long, conflicts_with = "resume")]
        renewal: bool,
        /// Quote resuming your unpaid suspended claim
        #[arg(long)]
        resume: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Claim a line with an accepted quote; charges the first 30-day rental now
    Claim(ClaimArgs),
    /// Release your claim: calls and SMS stop; the paid period is not refunded
    Release {
        line: String,
        /// Confirm the release
        #[arg(long)]
        yes: bool,
        #[arg(long, value_parser = parse_id)]
        request_id: Option<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Resume an unpaid suspended claim with a resume quote (new 30-day period)
    Resume(CoverageArgs),
    /// Renewal quote acceptance
    Renewal {
        #[command(subcommand)]
        command: RenewalCommands,
    },
    /// Merchant terms acceptance
    Terms {
        #[command(subcommand)]
        command: TermsCommands,
    },
    /// Standard voice and language of your claimed line (refused during a live call)
    Voice {
        #[command(subcommand)]
        command: VoiceCommands,
    },
    /// Booking confirmations texted to the current caller (at most two per call)
    Confirmations {
        #[command(subcommand)]
        command: ConfirmationCommands,
    },
}
#[derive(Args)]
pub struct LineArgs {
    /// Line ID
    pub line: String,
    #[command(flatten)]
    pub auth: AuthArgs,
}
#[derive(Clone, Copy, ValueEnum)]
pub enum VoiceTier {
    Standard,
    Premium,
}
#[derive(Args)]
pub struct ClaimArgs {
    /// Line ID
    pub line: String,
    /// Claim quote ID from `nyxid nyxbot line quote`
    #[arg(long, value_parser = parse_id)]
    pub quote: String,
    /// Specialist agent ID that answers this number
    #[arg(long, value_parser = parse_id)]
    pub agent: String,
    #[arg(long, value_enum, default_value = "standard")]
    pub voice_tier: VoiceTier,
    /// Voice from the quote's options (default: the line's default voice)
    #[arg(long, requires = "language")]
    pub voice: Option<String>,
    #[arg(long, requires = "voice")]
    pub language: Option<String>,
    /// Answer inbound SMS (requires approved SMS sender registration)
    #[arg(long)]
    pub sms: bool,
    /// Let the specialist text booking confirmations to callers
    #[arg(long, requires = "sms")]
    pub caller_confirmations: bool,
    /// Renew automatically at this price every 30 days
    #[arg(long)]
    pub auto_renew: bool,
    #[command(flatten)]
    pub consent: Consent,
    #[arg(long, value_parser = parse_id)]
    pub request_id: Option<String>,
    #[command(flatten)]
    pub auth: AuthArgs,
}
/// Explicit consent to the quoted terms versions. The server refuses any
/// mismatch with the quote and with the merchant's current required terms.
#[derive(Args)]
pub struct Consent {
    /// Managed line terms version you accept (shown by `quote`)
    #[arg(long = "accept-terms", value_name = "VERSION")]
    pub terms: String,
    /// ElevenLabs End User Agreement version, when the quote lists one
    #[arg(long = "accept-eua", value_name = "VERSION")]
    pub eua: Option<String>,
}
impl Consent {
    fn versions(&self) -> Value {
        json!({"managed_line": self.terms, "elevenlabs_eua": self.eua})
    }
}
#[derive(Args)]
pub struct CoverageArgs {
    /// Line ID
    pub line: String,
    /// Renewal or resume quote ID (renewal defaults to the pending renewal quote)
    #[arg(long, value_parser = parse_id)]
    pub quote: Option<String>,
    /// Turn automatic renewal on (default: keep the current setting)
    #[arg(long, conflicts_with = "no_auto_renew")]
    pub auto_renew: bool,
    /// Turn automatic renewal off
    #[arg(long)]
    pub no_auto_renew: bool,
    #[command(flatten)]
    pub consent: Consent,
    #[arg(long, value_parser = parse_id)]
    pub request_id: Option<String>,
    #[command(flatten)]
    pub auth: AuthArgs,
}
#[derive(Subcommand)]
pub enum RenewalCommands {
    /// Pay for the next 30 days with the renewal quote
    Accept(CoverageArgs),
}
#[derive(Subcommand)]
pub enum TermsCommands {
    /// Accept the current managed line terms (and End User Agreement)
    Accept {
        #[arg(long = "managed-line", value_name = "VERSION")]
        managed_line: String,
        #[arg(long, value_name = "VERSION")]
        eua: Option<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
}
#[derive(Subcommand)]
pub enum VoiceCommands {
    /// Choose a voice and language allowed on the line
    Set {
        line: String,
        #[arg(long)]
        voice: String,
        #[arg(long)]
        language: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
}
#[derive(Subcommand)]
pub enum ConfirmationCommands {
    /// Allow the specialist to text the current caller
    Enable(LineArgs),
    /// Stop caller confirmation texts
    Disable(LineArgs),
}

pub(crate) fn parse_id(value: &str) -> std::result::Result<String, String> {
    if Uuid::parse_str(value).is_ok_and(|id| id.get_version_num() == 4 && id.to_string() == value) {
        Ok(value.into())
    } else {
        Err("expected a canonical UUID v4".into())
    }
}
/// The request identity precedes network I/O so a lost response can be replayed.
pub(crate) fn request_id(given: Option<String>, kind: &str) -> String {
    let id = given.unwrap_or_else(|| Uuid::new_v4().to_string());
    eprintln!(
        "{}",
        json!({"event":"managed_line_request","kind":kind,"request_id":id,"retry":"rerun with --request-id to replay this request"})
    );
    id
}
fn print(value: &Value, output: OutputFormat, columns: &[(&str, &str)]) -> Result<()> {
    print_rows(value, output, None, columns)
}

async fn lines(api: &mut ApiClient) -> Result<Vec<Value>> {
    let mut all = Vec::new();
    let mut after: Option<String> = None;
    loop {
        let path = match &after {
            Some(a) => format!("/managed-lines?after={}", urlencoding::encode(a)),
            None => "/managed-lines".into(),
        };
        let page: Value = api.get(&path).await?;
        all.extend(page["lines"].as_array().cloned().unwrap_or_default());
        match page["next_after_id"].as_str() {
            Some(next) => after = Some(next.into()),
            None => return Ok(all),
        }
    }
}
async fn line(api: &mut ApiClient, id: &str) -> Result<Value> {
    lines(api)
        .await?
        .into_iter()
        .find(|l| l["id"].as_str() == Some(id))
        .context("Line not found among available numbers and your claims")
}
fn claim(line: &Value) -> Result<&Value> {
    line.get("claim")
        .filter(|c| !c.is_null())
        .context("You do not hold a claim on this line")
}
fn row(line: &Value) -> Value {
    let c = &line["claim"];
    json!({
        "id": line["id"],
        "phone_number": line["phone_number"],
        "country": line["country"],
        "state": line["state"],
        "voice": line["voice"],
        "sms": line["sms"],
        "claim_state": c["state"],
        "suspension": c["suspension"],
        "covered_from": c["covered_from"],
        "covered_until": c["covered_until"],
        "auto_renew": c["auto_renew"],
        "renewal_quote": c["renewal_quote"]["id"],
    })
}
const LINE_COLUMNS: &[(&str, &str)] = &[
    ("ID", "id"),
    ("Number", "phone_number"),
    ("Country", "country"),
    ("State", "state"),
    ("Voice", "voice"),
    ("SMS", "sms"),
    ("Claim", "claim_state"),
    ("Suspension", "suspension"),
    ("Covered from", "covered_from"),
    ("Covered until", "covered_until"),
];
fn quote_rows(q: &Value) -> Value {
    let mut rows = vec![json!({"item":"quote_id","value":q["id"]})];
    rows.push(json!({"item":"rental per day","value":q["rental"]["credits_per_unit"]}));
    rows.push(json!({"item":"period days","value":q["period_days"]}));
    for rate in q["usage"].as_array().into_iter().flatten() {
        let item = format!(
            "{} per {}",
            rate["purpose"].as_str().unwrap_or("usage"),
            rate["metric"].as_str().unwrap_or("unit")
        );
        rows.push(json!({"item":item,"value":rate["credits_per_unit"]}));
    }
    rows.push(json!({"item":"terms","value":q["terms"]["managed_line"]}));
    rows.push(json!({"item":"elevenlabs_eua","value":q["terms"]["elevenlabs_eua"]}));
    rows.push(json!({"item":"accept_before","value":q["accept_before"]}));
    rows.push(json!({"item":"paid period refundable","value":q["paid_period_refundable"]}));
    Value::Array(rows)
}

pub async fn run(command: LineCommands) -> Result<()> {
    match command {
        LineCommands::Eligibility(auth) => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let e: Value = api.get("/managed-lines/eligibility").await?;
            let view = json!({
                "business": e["business_legal_name"],
                "jurisdiction": e["jurisdiction"],
                "revision": e["revision"],
                "required_terms": e["required_terms"]["managed_line"],
                "required_eua": e["required_terms"]["elevenlabs_eua"],
                "accepted_terms": e["accepted_terms"]["managed_line"],
                "premium_allowed": e["premium_allowed"],
                "max_lines": e["max_lines"],
                "monthly_spend_limit": e["monthly_spend_limit"],
                "revoked_at": e["revoked_at"],
            });
            if matches!(auth.output, OutputFormat::Json) {
                return print(&e, auth.output, &[]);
            }
            print(
                &view,
                auth.output,
                &[
                    ("Business", "business"),
                    ("Revision", "revision"),
                    ("Terms required", "required_terms"),
                    ("EUA required", "required_eua"),
                    ("Terms accepted", "accepted_terms"),
                    ("Premium", "premium_allowed"),
                    ("Max lines", "max_lines"),
                    ("Spend limit", "monthly_spend_limit"),
                    ("Revoked", "revoked_at"),
                ],
            )
        }
        LineCommands::List(auth) => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let all = lines(&mut api).await?;
            if matches!(auth.output, OutputFormat::Json) {
                return print(&json!({"lines": all}), auth.output, &[]);
            }
            print(
                &Value::Array(all.iter().map(row).collect()),
                auth.output,
                LINE_COLUMNS,
            )
        }
        LineCommands::Show(args) => {
            let mut api = ApiClient::from_auth_checked(&args.auth).await?;
            let l = line(&mut api, &args.line).await?;
            if matches!(args.auth.output, OutputFormat::Json) {
                return print(&l, args.auth.output, &[]);
            }
            print(&row(&l), args.auth.output, LINE_COLUMNS)
        }
        LineCommands::Quote {
            line: id,
            renewal,
            resume,
            auth,
        } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let kind = if renewal || resume {
                let l = line(&mut api, &id).await?;
                let c = claim(&l)?;
                if renewal {
                    json!({"kind":"renewal","claim_id":c["id"],"generation":c["generation"],
                        "period_index":c["period_index"].as_i64().context("Coverage period is unavailable; update NyxID")?+1})
                } else {
                    json!({"kind":"resume","claim_id":c["id"],"generation":c["generation"]})
                }
            } else {
                json!({"kind":"claim"})
            };
            let q: Value = api
                .post(
                    &format!("/managed-lines/{}/quotes", urlencoding::encode(&id)),
                    &json!({ "kind": kind }),
                )
                .await?;
            if matches!(auth.output, OutputFormat::Json) {
                return print(&q, auth.output, &[]);
            }
            print(
                &quote_rows(&q),
                auth.output,
                &[("Item", "item"), ("Value", "value")],
            )
        }
        LineCommands::Claim(args) => {
            let mut api = ApiClient::from_auth_checked(&args.auth).await?;
            let standard_voice = match (&args.voice, &args.language) {
                (Some(voice), Some(language)) => json!({"voice":voice,"language":language}),
                _ => Value::Null,
            };
            let request = request_id(args.request_id, "claim");
            let body = json!({
                "request_id": request,
                "quote_id": args.quote,
                "line_id": args.line,
                "agent_id": args.agent,
                "voice_tier": match args.voice_tier { VoiceTier::Standard => "standard", VoiceTier::Premium => "premium" },
                "standard_voice": standard_voice,
                "enable_sms": args.sms,
                "caller_confirmations": args.caller_confirmations,
                "auto_renew": args.auto_renew,
                "accepted_terms_versions": args.consent.versions(),
            });
            let receipt: Value = api.post("/managed-lines/claims", &body).await?;
            print_receipt(&receipt, args.auth.output)
        }
        LineCommands::Release {
            line: id,
            yes,
            request_id: given,
            auth,
        } => {
            if !yes {
                bail!(
                    "Releasing stops calls and SMS now and the paid period is not refunded. Pass --yes to confirm."
                );
            }
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let l = line(&mut api, &id).await?;
            let claim_id = claim(&l)?["id"].clone();
            let request = request_id(given, "release");
            let receipt: Value = api
                .post(
                    "/managed-lines/releases",
                    &json!({"request_id":request,"line_id":id,"claim_id":claim_id}),
                )
                .await?;
            print_receipt(&receipt, auth.output)
        }
        LineCommands::Resume(args) => coverage(args, false).await,
        LineCommands::Renewal {
            command: RenewalCommands::Accept(args),
        } => coverage(args, true).await,
        LineCommands::Terms {
            command:
                TermsCommands::Accept {
                    managed_line,
                    eua,
                    auth,
                },
        } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let e: Value = api.get("/managed-lines/eligibility").await?;
            let revision = e["revision"].as_i64().context("Invalid eligibility")?;
            api.post_empty(
                "/managed-lines/eligibility/terms",
                &json!({"revision":revision,"terms":{"managed_line":managed_line,"elevenlabs_eua":eua}}),
            )
            .await?;
            done(auth.output, "Terms accepted.")
        }
        LineCommands::Voice {
            command:
                VoiceCommands::Set {
                    line: id,
                    voice,
                    language,
                    auth,
                },
        } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let l = line(&mut api, &id).await?;
            let c = claim(&l)?;
            let revision = c["voice_binding_revision"]
                .as_i64()
                .context("Voice binding revision is unavailable; update NyxID")?;
            api.put_empty(
                &format!("/managed-lines/{}/standard-voice", urlencoding::encode(&id)),
                &json!({"claim_id":c["id"],"binding_revision":revision,"option":{"voice":voice,"language":language}}),
            )
            .await?;
            done(auth.output, "Voice changed for the next call.")
        }
        LineCommands::Confirmations { command } => {
            let (args, enabled) = match command {
                ConfirmationCommands::Enable(args) => (args, true),
                ConfirmationCommands::Disable(args) => (args, false),
            };
            let mut api = ApiClient::from_auth_checked(&args.auth).await?;
            let l = line(&mut api, &args.line).await?;
            let c = claim(&l)?;
            let binding = c["voice_binding_id"]
                .as_str()
                .context("This line has no voice binding")?;
            let revision = c["voice_binding_revision"]
                .as_i64()
                .context("Voice binding revision is unavailable; update NyxID")?;
            api.put_empty(
                &format!(
                    "/call-adapters/bindings/{}/caller-confirmations",
                    urlencoding::encode(binding)
                ),
                &json!({"enabled":enabled,"sms_binding_id":null,"voice_revision":revision}),
            )
            .await?;
            done(
                args.auth.output,
                if enabled {
                    "Caller confirmations enabled."
                } else {
                    "Caller confirmations disabled."
                },
            )
        }
    }
}

async fn coverage(args: CoverageArgs, renewal: bool) -> Result<()> {
    let mut api = ApiClient::from_auth_checked(&args.auth).await?;
    let l = line(&mut api, &args.line).await?;
    let c = claim(&l)?;
    let quote = match args.quote {
        Some(q) => q,
        None if renewal => c["renewal_quote"]["id"]
            .as_str()
            .map(str::to_owned)
            .context(
                "No renewal quote is pending; run `nyxid nyxbot line quote <LINE> --renewal` first",
            )?,
        None => bail!("Pass --quote from `nyxid nyxbot line quote <LINE> --resume`"),
    };
    let auto_renew = if args.auto_renew {
        true
    } else if args.no_auto_renew {
        false
    } else {
        c["auto_renew"].as_bool().unwrap_or(false)
    };
    let request = request_id(args.request_id, if renewal { "renewal" } else { "resume" });
    let receipt: Value = api
        .post(
            "/managed-lines/coverage",
            &json!({"request_id":request,"quote_id":quote,"auto_renew":auto_renew,"accepted_terms_versions":args.consent.versions()}),
        )
        .await?;
    print_receipt(&receipt, args.auth.output)
}
pub(crate) fn print_receipt(receipt: &Value, output: OutputFormat) -> Result<()> {
    print(
        receipt,
        output,
        &[
            ("Request ID", "request_id"),
            ("Kind", "kind"),
            ("Line", "line_id"),
            ("Claim", "claim_id"),
            ("Period", "period_id"),
        ],
    )
}
pub(crate) fn done(output: OutputFormat, message: &str) -> Result<()> {
    if matches!(output, OutputFormat::Json) {
        println!("{}", json!({"ok": true}));
    } else {
        eprintln!("{message}");
    }
    Ok(())
}

#[cfg(test)]
#[path = "managed_line_tests.rs"]
mod tests;
