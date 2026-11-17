//! Platform-admin NyxID-managed phone lines. Inputs are files or flags without
//! secrets; the only secret (account rotation) is read from an env var or file.
use super::managed_line::{done, parse_id, print_receipt, request_id};
use crate::{
    api::ApiClient,
    cli::{AuthArgs, OutputFormat},
    output::print_rows,
};
use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand, ValueEnum};
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

#[derive(Subcommand)]
pub enum AdminLineCommands {
    /// List managed lines with state, claim and revision
    List(AuthArgs),
    /// Allocate a line ID and print the stable provider webhook URLs
    Prepare(AuthArgs),
    /// Register a prepared line after configuring its provider URLs (JSON/YAML file)
    Register {
        #[arg(long)]
        file: PathBuf,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Edit limits, regulatory evidence and tariffs; re-proves the provider resources
    Update {
        line: String,
        #[arg(long)]
        revision: i64,
        /// JSON/YAML with regulatory, supplier_tariffs and limits
        #[arg(long)]
        file: PathBuf,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Suspend an available or claimed line; live calls are asked to hang up
    Suspend(RevisionArgs),
    /// Resume an administratively suspended line
    Resume(RevisionArgs),
    /// Retire unclaimed inventory, or record a later supplier release
    Retire {
        line: String,
        #[arg(long)]
        revision: i64,
        /// RFC 3339 time the supplier stopped billing the number
        #[arg(long)]
        supplier_released_at: Option<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Return a cooled-down line to available inventory
    ApproveReassign(RevisionArgs),
    /// Release the current claim on a line (not refunded)
    Release {
        line: String,
        #[arg(long)]
        yes: bool,
        #[arg(long, value_parser = parse_id)]
        request_id: Option<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Set a line's allowed standard voice/language pairs (VOICE:LANGUAGE)
    Voices {
        line: String,
        #[arg(long)]
        revision: i64,
        #[arg(long = "option", value_name = "VOICE:LANGUAGE")]
        options: Vec<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Freeze provider seconds for a terminated call with permanently missing evidence
    ResolveEvidence {
        /// Call session ID
        session: String,
        #[arg(long)]
        provider_seconds: i64,
        #[arg(long)]
        provider_reference: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Estimated vs actual margin per line and UTC day (admin/operator)
    Report {
        /// UTC day (YYYY-MM-DD) or RFC 3339 midnight
        #[arg(long)]
        from: String,
        /// Exclusive UTC day; at most 31 days after --from
        #[arg(long)]
        until: String,
        #[arg(long)]
        line: Option<String>,
        /// Page the per-effect cost detail of --line instead of daily totals
        #[arg(long, requires = "line")]
        effects: bool,
        #[arg(long)]
        after: Option<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Merchant eligibility (person-owned)
    Eligibility {
        #[command(subcommand)]
        command: EligibilityCommands,
    },
    /// Trusted account rotation, underwriting, speech/premium, switch and cost policy
    Account {
        #[command(subcommand)]
        command: AccountCommands,
    },
}
#[derive(Args)]
pub struct RevisionArgs {
    pub line: String,
    #[arg(long)]
    pub revision: i64,
    #[command(flatten)]
    pub auth: AuthArgs,
}
#[derive(Args, Default)]
pub struct EligibilityFields {
    #[arg(long)]
    pub business_name: Option<String>,
    #[arg(long)]
    pub registration_id: Option<String>,
    /// ISO 3166-1 alpha-2 country
    #[arg(long)]
    pub jurisdiction: Option<String>,
    #[arg(long)]
    pub use_case: Option<String>,
    #[arg(long)]
    pub sms_sender_path: Option<String>,
    #[arg(long)]
    pub sms_registration_status: Option<String>,
    /// Managed line terms version merchants must accept
    #[arg(long)]
    pub terms_version: Option<String>,
    /// ElevenLabs End User Agreement version (required with premium)
    #[arg(long)]
    pub eua_version: Option<String>,
    #[arg(long)]
    pub premium_allowed: Option<bool>,
    #[arg(long)]
    pub max_lines: Option<i64>,
    /// Monthly pre-tax phone spend limit in credits
    #[arg(long, conflicts_with = "clear_monthly_spend_limit")]
    pub monthly_spend_limit: Option<String>,
    #[arg(long)]
    pub clear_monthly_spend_limit: bool,
}
#[derive(Subcommand)]
pub enum EligibilityCommands {
    /// List merchant eligibility records
    List(AuthArgs),
    /// Approve a person (UUID or email) as a managed-line merchant
    Grant {
        owner: String,
        #[command(flatten)]
        fields: EligibilityFields,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Change an approval; unspecified fields keep their current values
    Update {
        owner: String,
        #[arg(long)]
        revision: i64,
        #[command(flatten)]
        fields: EligibilityFields,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Apply an accepted eligibility revision to the merchant's claims
    Apply {
        owner: String,
        #[arg(long)]
        revision: i64,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Revoke eligibility: suspends every claim and stops new effects
    Revoke {
        owner: String,
        #[arg(long)]
        revision: i64,
        #[arg(long)]
        yes: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
}
#[derive(Clone, Copy, ValueEnum)]
pub enum Switch {
    On,
    Off,
}
#[derive(Subcommand)]
pub enum AccountCommands {
    /// List trusted accounts with underwriting and speech configuration
    List(AuthArgs),
    /// Rotate the account credential after proving the same provider account
    Rotate {
        account: String,
        #[arg(long)]
        revision: i64,
        /// Environment variable holding the new credential
        #[arg(
            long,
            conflicts_with = "credential_file",
            required_unless_present = "credential_file"
        )]
        credential_env: Option<String>,
        /// File holding the new credential (trailing newline trimmed)
        #[arg(long)]
        credential_file: Option<PathBuf>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Configure Telnyx underwriting ("underwritten exception, not reserved liability")
    Underwriting {
        account: String,
        #[arg(long)]
        revision: i64,
        #[arg(long)]
        file: PathBuf,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Configure per-call characters, greeting/wrap-up and premium ElevenLabs voices
    Premium {
        account: String,
        #[arg(long)]
        revision: i64,
        #[arg(long)]
        file: PathBuf,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Turn premium voices on or off for every managed line
    PremiumSwitch {
        #[arg(value_enum)]
        state: Switch,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Save a line's cost policy (FX, record schemas, supplier rental cycle)
    CostPolicy {
        /// Line ID (the policy is per line)
        line: String,
        #[arg(long)]
        revision: i64,
        #[arg(long)]
        file: PathBuf,
        #[command(flatten)]
        auth: AuthArgs,
    },
}

fn enc(value: &str) -> String {
    urlencoding::encode(value).into_owned()
}
fn document(path: &Path) -> Result<Map<String, Value>> {
    match super::admin_catalog::read_document(path)? {
        Value::Object(map) => Ok(map),
        _ => bail!("{} must contain an object", path.display()),
    }
}
fn with_revision(mut body: Map<String, Value>, revision: i64) -> Result<Value> {
    if body
        .get("revision")
        .is_some_and(|r| r.as_i64() != Some(revision))
    {
        bail!("The file's revision differs from --revision");
    }
    body.insert("revision".into(), json!(revision));
    Ok(Value::Object(body))
}
/// Exact credit amounts travel as decimal strings. YAML/JSON numbers are refused
/// locally instead of being rounded through floating point.
fn require_decimal_string(value: &Value, field: &str) -> Result<()> {
    if !value.is_string() {
        bail!("{field} must be a quoted decimal string, for example \"0.0002\"");
    }
    Ok(())
}
fn day(value: &str) -> String {
    if value.len() == 10 {
        format!("{value}T00:00:00Z")
    } else {
        value.into()
    }
}
const MAX_CREDENTIAL_BYTES: u64 = 65_536;
/// Drop trailing line endings in place; the removed bytes stay inside the same
/// zeroizing buffer instead of a separate unzeroized copy.
fn trim_line_endings(value: &mut String) {
    let len = value.trim_end_matches(['\r', '\n']).len();
    value.truncate(len);
}
fn read_secret_file(path: &Path) -> Result<Zeroizing<String>> {
    use std::io::Read;
    use zeroize::Zeroize;
    let file =
        std::fs::File::open(path).with_context(|| format!("Failed to read {}", path.display()))?;
    let size = file
        .metadata()
        .with_context(|| format!("Failed to read {}", path.display()))?
        .len();
    if size > MAX_CREDENTIAL_BYTES {
        bail!("The credential file exceeds {MAX_CREDENTIAL_BYTES} bytes");
    }
    // Reserve the whole file so reading never reallocates and leaves a stale
    // copy behind; the buffer is wiped on every exit path.
    let mut bytes = Zeroizing::new(Vec::with_capacity(size as usize + 1));
    file.take(MAX_CREDENTIAL_BYTES + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    if bytes.len() as u64 > MAX_CREDENTIAL_BYTES {
        bail!("The credential file exceeds {MAX_CREDENTIAL_BYTES} bytes");
    }
    // Moving the Vec into the String reuses the same allocation.
    let mut text = Zeroizing::new(String::from_utf8(std::mem::take(&mut *bytes)).map_err(
        |error| {
            let mut raw = error.into_bytes();
            raw.zeroize();
            anyhow::anyhow!("The credential file must be UTF-8")
        },
    )?);
    trim_line_endings(&mut text);
    Ok(text)
}
fn credential(env: Option<String>, file: Option<PathBuf>) -> Result<Zeroizing<String>> {
    let value = match (env, file) {
        (Some(name), None) => Zeroizing::new(
            std::env::var(&name).context("Credential environment variable is not set")?,
        ),
        (None, Some(path)) => read_secret_file(&path)?,
        _ => bail!("Provide exactly one of --credential-env or --credential-file"),
    };
    if value.trim().is_empty() {
        bail!("The credential is empty");
    }
    Ok(value)
}
/// Every page of an admin line or account listing: its rows, the stored
/// records the server skipped because they could not be read (accumulated
/// across pages, once each) and the first page's premium switch.
struct AdminListing {
    rows: Vec<Value>,
    unreadable_ids: Vec<String>,
    premium_enabled: Value,
}
async fn admin_listing(api: &mut ApiClient, path: &str, key: &str) -> Result<AdminListing> {
    let mut listing = AdminListing {
        rows: Vec::new(),
        unreadable_ids: Vec::new(),
        premium_enabled: Value::Null,
    };
    let mut after: Option<String> = None;
    loop {
        let url = match &after {
            Some(a) => format!("{path}?after={}", enc(a)),
            None => path.to_owned(),
        };
        let page: Value = api.get(&url).await?;
        if after.is_none() {
            listing.premium_enabled = page["premium_enabled"].clone();
        }
        listing
            .rows
            .extend(page[key].as_array().cloned().unwrap_or_default());
        for id in page["unreadable_ids"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            if !listing.unreadable_ids.iter().any(|seen| seen == id) {
                listing.unreadable_ids.push(id.to_owned());
            }
        }
        match page["next_after_id"].as_str() {
            Some(next) => after = Some(next.into()),
            None => return Ok(listing),
        }
    }
}
impl AdminListing {
    fn json(&self, key: &str) -> Value {
        json!({key:self.rows,"premium_enabled":self.premium_enabled,"unreadable_ids":self.unreadable_ids})
    }
}
/// The one-line table-output warning naming skipped records, if any.
fn unreadable_warning(ids: &[String]) -> Option<String> {
    (!ids.is_empty()).then(|| {
        format!(
            "Warning: {} stored record(s) could not be read and were skipped; repair them: {}",
            ids.len(),
            ids.join(", ")
        )
    })
}

async fn eligibility_rows(api: &mut ApiClient) -> Result<Vec<Value>> {
    let mut all = Vec::new();
    let mut after: Option<String> = None;
    loop {
        let path = match &after {
            Some(a) => format!("/admin/managed-comms/eligibility?after={}", enc(a)),
            None => "/admin/managed-comms/eligibility".into(),
        };
        let page: Value = api.get(&path).await?;
        all.extend(page["eligibility"].as_array().cloned().unwrap_or_default());
        match page["next_after_id"].as_str() {
            Some(next) => after = Some(next.into()),
            None => return Ok(all),
        }
    }
}
fn eligibility_body(
    current: Option<&Value>,
    f: EligibilityFields,
    expected: Option<i64>,
) -> Result<Value> {
    let text = |given: Option<String>, key: &str, flag: &str| -> Result<Value> {
        given
            .map(Value::String)
            .or_else(|| current.map(|c| c[key].clone()).filter(Value::is_string))
            .with_context(|| format!("{flag} is required"))
    };
    let terms = current.map(|c| &c["required_terms"]);
    let limit = if f.clear_monthly_spend_limit {
        Value::Null
    } else {
        f.monthly_spend_limit
            .map(Value::String)
            .or_else(|| current.map(|c| c["monthly_spend_limit"].clone()))
            .unwrap_or(Value::Null)
    };
    Ok(json!({
        "business_legal_name": text(f.business_name, "business_legal_name", "--business-name")?,
        "registration_id": text(f.registration_id, "registration_id", "--registration-id")?,
        "jurisdiction": text(f.jurisdiction, "jurisdiction", "--jurisdiction")?,
        "allowed_use_case": text(f.use_case, "allowed_use_case", "--use-case")?,
        "sms_sender_path": text(f.sms_sender_path, "sms_sender_path", "--sms-sender-path")?,
        "sms_registration_status": text(f.sms_registration_status, "sms_registration_status", "--sms-registration-status")?,
        "required_terms": {
            "managed_line": f.terms_version.map(Value::String)
                .or_else(|| terms.map(|t| t["managed_line"].clone()))
                .context("--terms-version is required")?,
            "elevenlabs_eua": f.eua_version.map(Value::String)
                .or_else(|| terms.map(|t| t["elevenlabs_eua"].clone()))
                .unwrap_or(Value::Null),
        },
        "premium_allowed": f.premium_allowed
            .or_else(|| current.and_then(|c| c["premium_allowed"].as_bool()))
            .unwrap_or(false),
        "max_lines": f.max_lines
            .or_else(|| current.and_then(|c| c["max_lines"].as_i64()))
            .unwrap_or(1),
        "monthly_spend_limit": limit,
        "expected_revision": expected,
    }))
}
const ELIGIBILITY_COLUMNS: &[(&str, &str)] = &[
    ("Owner", "owner_id"),
    ("Business", "business_legal_name"),
    ("Jurisdiction", "jurisdiction"),
    ("Revision", "revision"),
    ("Max lines", "max_lines"),
    ("Premium", "premium_allowed"),
    ("Spend limit", "monthly_spend_limit"),
    ("Revoked", "revoked_at"),
];
fn operation(value: &Value, output: OutputFormat) -> Result<()> {
    if value["termination_pending"] == true {
        eprintln!(
            "Termination pending for {} live calls; recovery keeps requesting hangups until the provider confirms.",
            value["live_calls"]
        );
    }
    print_rows(
        value,
        output,
        None,
        &[
            ("Line", "line_id"),
            ("Claim", "claim_id"),
            ("Revision", "revision"),
            ("State", "state"),
            ("Suspension", "suspension"),
            ("Live calls", "live_calls"),
        ],
    )
}

pub async fn run(command: AdminLineCommands) -> Result<()> {
    match command {
        AdminLineCommands::List(auth) => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let listing = admin_listing(&mut api, "/admin/managed-lines", "lines").await?;
            if matches!(auth.output, OutputFormat::Json) {
                return print_rows(&listing.json("lines"), auth.output, None, &[]);
            }
            if let Some(warning) = unreadable_warning(&listing.unreadable_ids) {
                eprintln!("{warning}");
            }
            let premium = &listing.premium_enabled;
            let rows: Vec<Value> = listing
                .rows
                .iter()
                .map(|l| {
                    json!({"id":l["id"],"phone_number":l["phone_number"],"provider":l["provider"],
                        "state":l["state"],"suspension":l["suspension"],"revision":l["revision"],
                        "claim":l["claim"]["id"],"owner":l["claim"]["owner_id"],"live_calls":l["live_calls"]})
                })
                .collect();
            eprintln!(
                "Premium voices: {}",
                if *premium == true { "on" } else { "off" }
            );
            print_rows(
                &Value::Array(rows),
                auth.output,
                None,
                &[
                    ("ID", "id"),
                    ("Number", "phone_number"),
                    ("Provider", "provider"),
                    ("State", "state"),
                    ("Suspension", "suspension"),
                    ("Revision", "revision"),
                    ("Claim", "claim"),
                    ("Owner", "owner"),
                    ("Live calls", "live_calls"),
                ],
            )
        }
        AdminLineCommands::Prepare(auth) => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let prepared: Value = api.post("/admin/managed-lines/prepare", &json!({})).await?;
            print_rows(
                &prepared,
                auth.output,
                None,
                &[
                    ("Line ID", "line_id"),
                    ("Voice URL", "voice_url"),
                    ("SMS URL", "sms_url"),
                ],
            )
        }
        AdminLineCommands::Register { file, auth } => {
            let body = Value::Object(document(&file)?);
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let line: Value = api.post("/admin/managed-lines", &body).await?;
            print_rows(
                &line,
                auth.output,
                None,
                &[("ID", "id"), ("State", "state"), ("Revision", "revision")],
            )
        }
        AdminLineCommands::Update {
            line,
            revision,
            file,
            auth,
        } => {
            let body = with_revision(document(&file)?, revision)?;
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let saved: Value = api
                .put(&format!("/admin/managed-lines/{}", enc(&line)), &body)
                .await?;
            print_rows(
                &saved,
                auth.output,
                None,
                &[("ID", "id"), ("State", "state"), ("Revision", "revision")],
            )
        }
        AdminLineCommands::Suspend(args) => line_operation(args, "suspend").await,
        AdminLineCommands::Resume(args) => line_operation(args, "resume").await,
        AdminLineCommands::Retire {
            line,
            revision,
            supplier_released_at,
            auth,
        } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let result: Value = api
                .post(
                    &format!("/admin/managed-lines/{}/retire", enc(&line)),
                    &json!({"revision":revision,"supplier_released_at":supplier_released_at}),
                )
                .await?;
            operation(&result, auth.output)
        }
        AdminLineCommands::ApproveReassign(args) => {
            let mut api = ApiClient::from_auth_checked(&args.auth).await?;
            api.post_empty(
                &format!(
                    "/admin/managed-lines/{}/approve-reassignment",
                    enc(&args.line)
                ),
                &json!({"revision":args.revision}),
            )
            .await?;
            done(args.auth.output, "Line returned to available inventory.")
        }
        AdminLineCommands::Release {
            line,
            yes,
            request_id: given,
            auth,
        } => {
            if !yes {
                bail!(
                    "Releasing stops the merchant's calls and SMS; the paid period is not refunded. Pass --yes to confirm."
                );
            }
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let page: Value = api.get("/admin/managed-lines").await?;
            let mut claim = page["lines"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|l| l["id"].as_str() == Some(line.as_str()))
                .map(|l| l["claim"]["id"].clone());
            let mut after = page["next_after_id"].as_str().map(str::to_owned);
            while claim.is_none() {
                let Some(cursor) = after.take() else { break };
                let page: Value = api
                    .get(&format!("/admin/managed-lines?after={}", enc(&cursor)))
                    .await?;
                claim = page["lines"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|l| l["id"].as_str() == Some(line.as_str()))
                    .map(|l| l["claim"]["id"].clone());
                after = page["next_after_id"].as_str().map(str::to_owned);
            }
            let claim = claim
                .filter(Value::is_string)
                .context("This line has no current claim")?;
            let request = request_id(given, "admin_release");
            let receipt: Value = api
                .post(
                    "/admin/managed-lines/releases",
                    &json!({"request_id":request,"line_id":line,"claim_id":claim}),
                )
                .await?;
            print_receipt(&receipt, auth.output)
        }
        AdminLineCommands::Voices {
            line,
            revision,
            options,
            auth,
        } => {
            let options = options
                .iter()
                .map(|o| {
                    let (voice, language) =
                        o.rsplit_once(':').context("Use --option VOICE:LANGUAGE")?;
                    Ok(json!({"voice":voice,"language":language}))
                })
                .collect::<Result<Vec<_>>>()?;
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            api.put_empty(
                &format!("/admin/managed-lines/{}/standard-voices", enc(&line)),
                &json!({"revision":revision,"options":options}),
            )
            .await?;
            done(auth.output, "Standard voices saved.")
        }
        AdminLineCommands::ResolveEvidence {
            session,
            provider_seconds,
            provider_reference,
            auth,
        } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            api.post_empty(
                &format!("/admin/managed-lines/calls/{}/resolve-evidence", enc(&session)),
                &json!({"provider_seconds":provider_seconds,"provider_reference":provider_reference}),
            )
            .await?;
            done(auth.output, "Evidence recorded and the call settled.")
        }
        AdminLineCommands::Report {
            from,
            until,
            line,
            effects,
            after,
            auth,
        } => {
            let mut query = format!("from={}&until={}", enc(&day(&from)), enc(&day(&until)));
            if let Some(line) = &line {
                query.push_str(&format!("&line_id={}", enc(line)));
            }
            if let Some(after) = &after {
                query.push_str(&format!("&after={}", enc(after)));
            }
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let path = if effects {
                format!("/admin/managed-lines/report/effects?{query}")
            } else {
                format!("/admin/managed-lines/report?{query}")
            };
            let report: Value = api.get(&path).await?;
            if matches!(auth.output, OutputFormat::Json) {
                return print_rows(&report, auth.output, None, &[]);
            }
            if let Some(next) = report["next_after"].as_str() {
                eprintln!("More results: --after {next}");
            }
            if effects {
                return print_rows(
                    &report,
                    auth.output,
                    Some("effects"),
                    &[
                        ("Effect", "effect_id"),
                        ("Kind", "kind"),
                        ("Created", "created_at"),
                        ("Coverage", "component_coverage"),
                        ("Evidence", "evidence_quality"),
                    ],
                );
            }
            let rows: Vec<Value> = report["lines"]
                .as_array()
                .into_iter()
                .flatten()
                .flat_map(|l| {
                    l["days"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default()
                        .into_iter()
                        .map(|mut d| {
                            d["line_id"] = l["line_id"].clone();
                            d
                        })
                })
                .collect();
            print_rows(
                &Value::Array(rows),
                auth.output,
                None,
                &[
                    ("Line", "line_id"),
                    ("Day", "day"),
                    ("Effects", "effect_count"),
                    ("Billed", "billed_credits"),
                    ("Estimated cost", "estimated_cost_credits"),
                    ("Provider cost", "provider_actual_cost_credits"),
                    ("Inventory", "inventory_cost_credits"),
                    ("Estimated margin", "estimated_margin_credits"),
                    ("Actual margin", "actual_margin_credits"),
                    ("Unknown", "unknown_components"),
                    ("Absorbed s", "absorbed_seconds"),
                ],
            )
        }
        AdminLineCommands::Eligibility { command } => eligibility(command).await,
        AdminLineCommands::Account { command } => account(command).await,
    }
}

async fn line_operation(args: RevisionArgs, path: &str) -> Result<()> {
    let mut api = ApiClient::from_auth_checked(&args.auth).await?;
    let result: Value = api
        .post(
            &format!("/admin/managed-lines/{}/{path}", enc(&args.line)),
            &json!({"revision":args.revision}),
        )
        .await?;
    operation(&result, args.auth.output)
}

async fn eligibility(command: EligibilityCommands) -> Result<()> {
    match command {
        EligibilityCommands::List(auth) => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let rows = eligibility_rows(&mut api).await?;
            print_rows(&Value::Array(rows), auth.output, None, ELIGIBILITY_COLUMNS)
        }
        EligibilityCommands::Grant {
            owner,
            fields,
            auth,
        } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let owner = super::admin::resolve_user_id(&mut api, &owner).await?;
            let body = eligibility_body(None, fields, None)?;
            let saved: Value = api
                .put(
                    &format!("/admin/managed-comms/eligibility/{}", enc(&owner)),
                    &body,
                )
                .await?;
            print_rows(&saved, auth.output, None, ELIGIBILITY_COLUMNS)
        }
        EligibilityCommands::Update {
            owner,
            revision,
            fields,
            auth,
        } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let owner = super::admin::resolve_user_id(&mut api, &owner).await?;
            let current = eligibility_rows(&mut api)
                .await?
                .into_iter()
                .find(|r| r["owner_id"].as_str() == Some(owner.as_str()))
                .context("This person has no managed-line eligibility; use grant")?;
            let body = eligibility_body(Some(&current), fields, Some(revision))?;
            let saved: Value = api
                .put(
                    &format!("/admin/managed-comms/eligibility/{}", enc(&owner)),
                    &body,
                )
                .await?;
            print_rows(&saved, auth.output, None, ELIGIBILITY_COLUMNS)
        }
        EligibilityCommands::Apply {
            owner,
            revision,
            auth,
        } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let owner = super::admin::resolve_user_id(&mut api, &owner).await?;
            api.post_empty(
                &format!("/admin/managed-comms/eligibility/{}/apply", enc(&owner)),
                &json!({"revision":revision}),
            )
            .await?;
            done(auth.output, "Revision applied to the merchant's claims.")
        }
        EligibilityCommands::Revoke {
            owner,
            revision,
            yes,
            auth,
        } => {
            if !yes {
                bail!("Revocation suspends every claim of this merchant. Pass --yes to confirm.");
            }
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let owner = super::admin::resolve_user_id(&mut api, &owner).await?;
            api.post_empty(
                &format!("/admin/managed-comms/eligibility/{}/revoke", enc(&owner)),
                &json!({"revision":revision}),
            )
            .await?;
            done(auth.output, "Eligibility revoked; claims are suspended.")
        }
    }
}

async fn account(command: AccountCommands) -> Result<()> {
    match command {
        AccountCommands::List(auth) => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let listing =
                admin_listing(&mut api, "/admin/managed-lines/accounts", "accounts").await?;
            if matches!(auth.output, OutputFormat::Json) {
                return print_rows(&listing.json("accounts"), auth.output, None, &[]);
            }
            if let Some(warning) = unreadable_warning(&listing.unreadable_ids) {
                eprintln!("{warning}");
            }
            let rows: Vec<Value> = listing
                .rows
                .iter()
                .map(|a| {
                    json!({"id":a["id"],"provider":a["provider"],"account_id":a["account_id"],"revision":a["revision"],
                        "underwriting":a["underwriting"]["revision"],"underwriting_enabled":a["underwriting"]["enabled"],
                        "voice":a["voice"]["revision"],"premium":a["voice"]["premium"]["plan_tier"]})
                })
                .collect();
            print_rows(
                &Value::Array(rows),
                auth.output,
                None,
                &[
                    ("ID", "id"),
                    ("Provider", "provider"),
                    ("Account", "account_id"),
                    ("Revision", "revision"),
                    ("Underwriting rev", "underwriting"),
                    ("Underwriting on", "underwriting_enabled"),
                    ("Voice rev", "voice"),
                    ("Premium tier", "premium"),
                ],
            )
        }
        AccountCommands::Rotate {
            account,
            revision,
            credential_env,
            credential_file,
            auth,
        } => {
            let secret = credential(credential_env, credential_file)?;
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            // The successor credential travels only over HTTPS or exact loopback.
            api.require_credential_transport()?;
            // Borrow the zeroizing secret directly into the request body.
            #[derive(serde::Serialize)]
            struct Rotation<'a> {
                expected_revision: i64,
                credential: &'a str,
            }
            api.post_empty(
                &format!("/admin/managed-comms/accounts/{}/rotate", enc(&account)),
                &Rotation {
                    expected_revision: revision,
                    credential: secret.as_str(),
                },
            )
            .await?;
            done(auth.output, "Credential proved and rotated.")
        }
        AccountCommands::Underwriting {
            account,
            revision,
            file,
            auth,
        } => {
            let body = with_revision(document(&file)?, revision)?;
            for component in body["components"].as_array().into_iter().flatten() {
                require_decimal_string(
                    &component["credits_per_second"],
                    "components[].credits_per_second",
                )?;
            }
            require_decimal_string(&body["rolling_budget"], "rolling_budget")?;
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            api.put_empty(
                &format!(
                    "/admin/managed-lines/accounts/{}/underwriting",
                    enc(&account)
                ),
                &body,
            )
            .await?;
            done(auth.output, "Underwriting saved.")
        }
        AccountCommands::Premium {
            account,
            revision,
            file,
            auth,
        } => {
            let body = with_revision(document(&file)?, revision)?;
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            api.put_empty(
                &format!("/admin/managed-lines/accounts/{}/premium", enc(&account)),
                &body,
            )
            .await?;
            done(auth.output, "Speech and premium voice configuration saved.")
        }
        AccountCommands::PremiumSwitch { state, auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let enabled = matches!(state, Switch::On);
            api.put_empty(
                "/admin/managed-lines/premium-switch",
                &json!({ "enabled": enabled }),
            )
            .await?;
            done(
                auth.output,
                if enabled {
                    "Premium voices enabled."
                } else {
                    "Premium voices disabled; live premium calls wrap up and end."
                },
            )
        }
        AccountCommands::CostPolicy {
            line,
            revision,
            file,
            auth,
        } => {
            // Sent as written: `inventory.cycle_anchor` is an RFC 3339 string.
            let body = with_revision(document(&file)?, revision)?;
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            api.put_empty(
                &format!("/admin/managed-lines/{}/cost-policy", enc(&line)),
                &body,
            )
            .await?;
            done(auth.output, "Cost policy saved.")
        }
    }
}

#[cfg(test)]
#[path = "admin_line_tests.rs"]
mod tests;
