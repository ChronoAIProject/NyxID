//! Public model discovery and human-only, revision-fenced administration.
use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use super::billing_support as support;
use crate::{api::ApiClient, cli::AuthArgs};

#[derive(Subcommand)]
pub enum ModelCommands {
    /// List entitled models, public capabilities and exact retail prices
    List(PageArgs),
    /// Find an entitled public ID (nyxid/model-name); never uses admin routes
    Show {
        model: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
}
#[derive(Args)]
pub struct PageArgs {
    #[arg(long, default_value_t = 50, value_parser = clap::value_parser!(u16).range(1..=100))]
    limit: u16,
    #[arg(long)]
    after: Option<String>,
    #[command(flatten)]
    auth: AuthArgs,
}
#[derive(Args)]
pub struct ModelArgs {
    model: String,
    #[command(flatten)]
    auth: AuthArgs,
}
#[derive(Args)]
pub struct SetArgs {
    model: String,
    /// Current MODEL revision; stale writes fail without retry
    #[arg(long)]
    revision: i64,
    /// JSON object (routes/entitlements wrapper for set; complete contract for update)
    #[arg(long)]
    file: PathBuf,
    #[command(flatten)]
    auth: AuthArgs,
}
#[derive(Subcommand)]
pub enum AdminModelCommands {
    List(PageArgs),
    Show(ModelArgs),
    /// Create a disabled draft from a public-contract JSON file
    Create {
        #[arg(long)]
        file: PathBuf,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Replace mutable public contract, lifecycle and enable state
    Update(SetArgs),
    /// Retire the model permanently; public IDs cannot be reused
    Retire {
        model: String,
        #[arg(long)]
        revision: i64,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Inspect or replace private same-model routes
    Routes {
        #[command(subcommand)]
        command: RoutesCommands,
    },
    /// Inspect or replace default-deny policy; any matching deny wins
    Access {
        #[command(subcommand)]
        command: AccessCommands,
    },
    /// Immutable exact tariffs, publication and billing readiness
    Prices {
        #[command(subcommand)]
        command: PriceCommands,
    },
}
#[derive(Subcommand)]
pub enum RoutesCommands {
    Show(ModelArgs),
    Set(SetArgs),
}
#[derive(Subcommand)]
pub enum AccessCommands {
    Show(ModelArgs),
    Set(SetArgs),
    Explain {
        model: String,
        #[arg(long)]
        user: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
}
#[derive(Subcommand)]
pub enum PriceCommands {
    List(ModelArgs),
    /// Create a pending tariff from certified contract/bounds/components JSON
    Create(SetArgs),
    /// Publish all complete pending revisions via existing billing resync
    Publish(ModelArgs),
    Resync(ModelArgs),
    /// Explain readiness without a provider request
    Verify(ModelArgs),
    Retire {
        model: String,
        tariff: String,
        #[arg(long)]
        revision: i64,
        #[command(flatten)]
        auth: AuthArgs,
    },
}
fn base(model: &str) -> Result<String> {
    Ok(format!("/admin/platform-models/{}", support::uuid(model)?))
}
fn page_path(admin: bool, limit: u16, after: Option<&str>) -> String {
    let base = if admin {
        "/admin/platform-models"
    } else {
        "/llm/models"
    };
    format!(
        "{base}?limit={limit}{}",
        after
            .map(|s| format!("&after={}", urlencoding::encode(s)))
            .unwrap_or_default()
    )
}
fn pick(row: &Value, keys: &[&str]) -> Value {
    Value::Object(
        keys.iter()
            .filter_map(|k| row.get(*k).map(|v| ((*k).into(), v.clone())))
            .collect(),
    )
}
/// Defense in depth even against a server accidentally returning private fields.
fn public_model(row: &Value) -> Value {
    let mut result = pick(row, &["id", "display_name", "description", "availability"]);
    result["capabilities"] = pick(
        &row["capabilities"],
        &[
            "protocols",
            "input_modalities",
            "output_modalities",
            "context_tokens",
            "output_tokens",
            "streaming",
            "tools",
            "structured_output",
        ],
    );
    result["retail_prices"] = json!(
        row["retail_prices"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|price| {
                let mut safe = pick(price, &["variant", "available"]);
                safe["components"] = json!(
                    price["components"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|c| {
                            let mut c = pick(
                                c,
                                &["metric", "credits_per_unit", "explicit_free", "selector"],
                            );
                            c["selector"] = pick(&c["selector"], &["kind", "ttl", "modality"]);
                            c
                        })
                        .collect::<Vec<_>>()
                );
                safe
            })
            .collect::<Vec<_>>()
    );
    result
}
fn public_page(page: Value) -> Value {
    json!({"models": page["models"].as_array().into_iter().flatten().map(public_model).collect::<Vec<_>>(), "next_after": page["next_after"]})
}
fn render(value: &Value) -> String {
    if let Some(rows) = value["models"].as_array() {
        if rows.iter().any(|r| r.get("public_id").is_some()) {
            let table = support::table(
                &["ID", "Public model", "Name", "State", "Revision"],
                rows.iter().map(|r| {
                    ["id", "public_id", "display_name", "lifecycle", "revision"]
                        .iter()
                        .map(|k| support::text(&r[*k]))
                        .collect()
                }),
            );
            return format!(
                "{table}\nNext cursor: {}",
                support::text(&value["next_after"])
            );
        }
        let table = support::table(
            &["Model", "Name", "State"],
            rows.iter().map(|r| {
                vec![
                    support::text(r.get("public_id").unwrap_or(&r["id"])),
                    support::text(&r["display_name"]),
                    support::text(r.get("availability").unwrap_or(&r["lifecycle"])),
                ]
            }),
        );
        let prices = rows
            .iter()
            .filter(|r| r.get("retail_prices").is_some())
            .map(render_model)
            .collect::<Vec<_>>()
            .join("\n");
        return format!(
            "{table}\n{prices}\nNext cursor: {}",
            support::text(&value["next_after"])
        );
    }
    if value.get("retail_prices").is_some() {
        return render_model(value);
    }
    // Admin output preserves exact strings and unknown future units.
    serde_json::to_string_pretty(value).unwrap_or_default()
}
fn render_model(row: &Value) -> String {
    let mut lines = vec![format!(
        "{} — {} ({})",
        support::text(&row["id"]),
        support::text(&row["display_name"]),
        support::text(&row["availability"])
    )];
    if let Some(description) = row["description"].as_str() {
        lines.push(description.to_owned());
    }
    if row["capabilities"]
        .as_object()
        .is_some_and(|c| !c.is_empty())
    {
        lines.push(support::fields(&row["capabilities"]));
    }
    let prices: Vec<_> = row["retail_prices"]
        .as_array()
        .into_iter()
        .flatten()
        .collect();
    if prices.is_empty() {
        lines.push("Retail prices unavailable".into());
    }
    for p in prices {
        lines.push(format!("Variant: {}", p["variant"]));
        if p["available"] != true {
            lines.push("Retail prices unavailable".into());
            continue;
        }
        for c in p["components"].as_array().into_iter().flatten() {
            let rate = if c["explicit_free"] == true {
                "Free".into()
            } else {
                format!("{} credits / unit", support::text(&c["credits_per_unit"]))
            };
            lines.push(format!(
                "  {} {}: {rate}",
                support::text(&c["metric"]),
                c["selector"]
            ));
        }
    }
    lines.join("\n")
}
pub async fn run(command: ModelCommands) -> Result<()> {
    let (value, auth) = match command {
        ModelCommands::List(args) => {
            let mut api = ApiClient::from_auth_checked(&args.auth).await?;
            (
                public_page(
                    api.get(&page_path(false, args.limit, args.after.as_deref()))
                        .await?,
                ),
                args.auth,
            )
        }
        ModelCommands::Show { model, auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            (find_public(&mut api, &model).await?, auth)
        }
    };
    support::print(&value, auth.output, || render(&value))
}
async fn find_public(api: &mut ApiClient, model: &str) -> Result<Value> {
    if !model.starts_with("nyxid/") {
        bail!("Use the public nyxid/model-name ID");
    }
    let mut after = None;
    let mut seen = HashSet::new();
    loop {
        let page: Value = api.get(&page_path(false, 100, after.as_deref())).await?;
        if let Some(row) = page["models"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|r| r["id"] == model)
        {
            return Ok(public_model(row));
        }
        let Some(next) = page["next_after"].as_str() else {
            bail!("Model unavailable");
        };
        if !seen.insert(next.to_string()) {
            bail!("Model listing returned a repeated cursor");
        }
        after = Some(next.to_string());
    }
}
fn manifest(file: &Path, revision: Option<(&str, i64)>) -> Result<Value> {
    let mut value: Value =
        serde_json::from_slice(&std::fs::read(file).context("Read model manifest")?)
            .context("Parse model manifest JSON")?;
    let object = value
        .as_object_mut()
        .context("Manifest must be a JSON object")?;
    if let Some((key, rev)) = revision {
        if rev < 1 {
            bail!("Revision must be positive");
        }
        if object.contains_key("revision") || object.contains_key("expected_revision") {
            bail!("Pass the revision only via --revision");
        }
        object.insert(key.into(), json!(rev));
    }
    Ok(value)
}
async fn write(args: SetArgs, suffix: &str, post: bool) -> Result<(Value, AuthArgs)> {
    let path = format!("{}{suffix}", base(&args.model)?);
    let body = manifest(
        &args.file,
        Some((
            if post {
                "revision"
            } else {
                "expected_revision"
            },
            args.revision,
        )),
    )?;
    let mut api = support::human_api(&args.auth).await?;
    let value = if post {
        api.post(&path, &body).await?
    } else {
        api.put(&path, &body).await?
    };
    Ok((value, args.auth))
}
pub async fn run_admin(command: AdminModelCommands) -> Result<()> {
    let (value, auth) = match command {
        AdminModelCommands::List(args) => {
            let mut api = support::human_api(&args.auth).await?;
            (
                api.get::<Value>(&page_path(true, args.limit, args.after.as_deref()))
                    .await?,
                args.auth,
            )
        }
        AdminModelCommands::Show(args) => get(args, "").await?,
        AdminModelCommands::Create { file, auth } => {
            let body = manifest(&file, None)?;
            let mut api = support::human_api(&auth).await?;
            (
                api.post::<Value, _>("/admin/platform-models", &body)
                    .await?,
                auth,
            )
        }
        AdminModelCommands::Update(args) => write(args, "", false).await?,
        AdminModelCommands::Retire {
            model,
            revision,
            auth,
        } => {
            let path = base(&model)?;
            let mut api = support::human_api(&auth).await?;
            let row: Value = api.get(&path).await?;
            if row["revision"] != revision {
                bail!("Model changed; inspect the current revision before retrying");
            }
            let mut body = pick(
                &row,
                &[
                    "display_name",
                    "description",
                    "capabilities",
                    "billing_service_id",
                ],
            );
            body["expected_revision"] = json!(revision);
            body["lifecycle"] = json!("retired");
            body["enabled"] = json!(false);
            (api.put::<Value, _>(&path, &body).await?, auth)
        }
        AdminModelCommands::Routes { command } => match command {
            RoutesCommands::Show(args) => {
                let (v, a) = get(args, "").await?;
                (
                    pick(
                        &v,
                        &[
                            "id",
                            "public_id",
                            "revision",
                            "routing_revision_id",
                            "routes",
                        ],
                    ),
                    a,
                )
            }
            RoutesCommands::Set(args) => write(args, "/routes", false).await?,
        },
        AdminModelCommands::Access { command } => match command {
            AccessCommands::Show(args) => {
                let (v, a) = get(args, "").await?;
                (
                    pick(&v, &["id", "public_id", "revision", "entitlements"]),
                    a,
                )
            }
            AccessCommands::Set(args) => write(args, "/entitlements", false).await?,
            AccessCommands::Explain { model, user, auth } => {
                let path = format!("{}/access?user_id={}", base(&model)?, support::uuid(&user)?);
                let mut api = support::human_api(&auth).await?;
                (api.get::<Value>(&path).await?, auth)
            }
        },
        AdminModelCommands::Prices { command } => match command {
            PriceCommands::List(args) => get(args, "/tariffs").await?,
            PriceCommands::Create(args) => write(args, "/tariffs", true).await?,
            PriceCommands::Publish(args) | PriceCommands::Resync(args) => {
                billing(args, true).await?
            }
            PriceCommands::Verify(args) => billing(args, false).await?,
            PriceCommands::Retire {
                model,
                tariff,
                revision,
                auth,
            } => {
                let path = format!(
                    "{}/tariffs/{}/retire",
                    base(&model)?,
                    support::uuid(&tariff)?
                );
                let mut api = support::human_api(&auth).await?;
                api.post_empty(&path, &json!({"revision":revision})).await?;
                (json!({"retired":true}), auth)
            }
        },
    };
    support::print(&value, auth.output, || render(&value))
}
async fn get(args: ModelArgs, suffix: &str) -> Result<(Value, AuthArgs)> {
    let path = format!("{}{suffix}", base(&args.model)?);
    let mut api = support::human_api(&args.auth).await?;
    Ok((api.get(&path).await?, args.auth))
}
async fn billing(args: ModelArgs, sync: bool) -> Result<(Value, AuthArgs)> {
    let mut api = support::human_api(&args.auth).await?;
    let row: Value = api.get(&base(&args.model)?).await?;
    let service = support::uuid(
        row["billing_service_id"]
            .as_str()
            .context("Model has no logical billing service")?,
    )?;
    let path = format!("/admin/billing/services/{service}");
    let value: Value = if sync {
        api.post(&format!("{path}/resync"), &json!({})).await?
    } else {
        api.get(&path).await?
    };
    Ok((
        json!({"model":pick(&row, &["public_id","execution_ready","blocking_reason","required_readiness"]),"billing":value}),
        args.auth,
    ))
}

#[cfg(test)]
mod tests;
