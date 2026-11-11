//! Authoring uses server-certified metadata; it never infers prices or bounds.
use super::*;

#[derive(Subcommand)]
pub enum ContractCommands {
    List {
        #[command(flatten)]
        auth: AuthArgs,
    },
}
#[derive(Args)]
pub struct PriceCreateArgs {
    #[command(flatten)]
    pub args: SetArgs,
    /// Certified family contract (see contracts list); fills version and fixture digest
    #[arg(long, requires = "attest")]
    pub contract: Option<String>,
    /// Confirms verification of model limits, features and prices; link or note
    #[arg(long, requires = "contract")]
    pub attest: Option<String>,
}
const PATH: &str = "/admin/platform-models/contracts";
pub async fn run(command: ContractCommands) -> Result<()> {
    let ContractCommands::List { auth } = command;
    let mut api = support::human_api(&auth).await?;
    let value: Value = api.get(PATH).await?;
    support::print(&value, auth.output, || {
        support::table(
            &[
                "Version",
                "Family",
                "Transports",
                "Fixture SHA-256",
                "Allowed options",
            ],
            value.as_array().into_iter().flatten().map(|d| {
                vec![
                    support::text(&d["version"]),
                    support::text(&d["family"]),
                    d["transports"].to_string(),
                    support::text(&d["fixture_sha256"]),
                    pick(
                        d,
                        &[
                            "flags",
                            "cache_write_ttls",
                            "input_modalities",
                            "output_modalities",
                        ],
                    )
                    .to_string(),
                ]
            }),
        )
    })
}
pub async fn create(input: PriceCreateArgs) -> Result<(Value, AuthArgs)> {
    let Some(version) = input.contract else {
        return write(input.args, "/tariffs", true).await;
    };
    let args = input.args;
    let path = base(&args.model)?;
    let mut body = manifest(&args.file, Some(("revision", args.revision)))?;
    let reference = input.attest.context("--contract requires --attest")?;
    if reference.trim().is_empty() || reference.chars().any(char::is_control) {
        bail!("Provide a nonempty verification reference without control characters");
    }
    let mut api = support::human_api(&args.auth).await?;
    let definitions: Vec<Value> = api.get(PATH).await?;
    let definition = definitions
        .iter()
        .find(|d| d["version"] == version)
        .context("Unknown certified contract; run admin models contracts list")?;
    let model: Value = api.get(&path).await?;
    if model["revision"] != args.revision {
        bail!("Model changed; inspect the current revision before retrying");
    }
    if model["family"] != definition["family"] {
        bail!("Contract family does not match the model");
    }
    let source = format!(
        "contract-attested-v1:{} {}",
        model["public_id"]
            .as_str()
            .context("Model has no public ID")?,
        reference.trim()
    );
    if source.len() > 1000 {
        bail!("Verification source must be at most 1000 UTF-8 bytes");
    }
    let mut contract = definition["default_contract"]
        .as_object()
        .context("Invalid contract metadata")?
        .clone();
    if let Some(overrides) = body.get("contract") {
        let overrides = overrides
            .as_object()
            .context("Contract must be a JSON object")?;
        for (key, value) in overrides {
            if !contract.contains_key(key) {
                bail!("Unknown contract option: {key}");
            }
            if ["version", "fixture_sha256", "family"].contains(&key.as_str()) {
                continue;
            }
            contract.insert(key.clone(), value.clone());
        }
    }
    for (field, rule) in definition["flags"]
        .as_object()
        .context("Invalid contract flags")?
    {
        let value = contract[field]
            .as_bool()
            .context("Contract flags must be boolean")?;
        if (rule == "required" && !value) || (rule == "forbidden" && value) {
            bail!("Unsupported contract option: {field}");
        }
    }
    for field in ["cache_write_ttls", "input_modalities", "output_modalities"] {
        let selected = contract[field]
            .as_array()
            .context("Contract options must be arrays")?;
        let allowed = definition[field]
            .as_array()
            .context("Invalid contract options")?;
        if selected.iter().any(|v| !allowed.contains(v))
            || selected
                .iter()
                .enumerate()
                .any(|(i, v)| selected[..i].contains(v))
        {
            bail!("Unsupported or duplicate contract option: {field}");
        }
    }
    body["contract"] = Value::Object(contract);
    body["source"] = json!(source);
    let response = api.post(&format!("{path}/tariffs"), &body).await?;
    Ok((response, args.auth))
}
