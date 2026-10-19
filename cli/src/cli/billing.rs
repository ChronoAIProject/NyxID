use super::{AuthArgs, PathBuf};
use clap::{Args, Subcommand};

#[derive(Subcommand)]
pub enum AdminBillingCommands {
    /// Read or change global billing defaults
    Config {
        #[command(subcommand)]
        command: BillingConfigCommands,
    },
    /// Show effective prices for every billed service
    Overview {
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Inspect the exact Lago rate cache
    Rates {
        #[arg(long)]
        service: Option<String>,
        #[arg(long)]
        include_retired: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Inspect and configure one catalog service's billing
    Service {
        #[command(subcommand)]
        command: BillingServiceCommands,
    },
    /// Verify the append-only billing ledger
    Ledger {
        #[command(subcommand)]
        command: BillingLedgerCommands,
    },
}

#[derive(Subcommand)]
pub enum BillingConfigCommands {
    /// Show billing availability, environment and overdraft defaults
    Show {
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Set the whole-credit default for newly provisioned wallets
    Set {
        /// 0 through 1000000000000, or default to clear the global override
        #[arg(long)]
        default_overdraft_cap_credits: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
}

// Clap's flattened argument structs cannot be boxed; parsed once per invocation.
#[allow(clippy::large_enum_variant)]
#[derive(Subcommand)]
pub enum BillingServiceCommands {
    /// Join effective prices with raw lanes and the managed contract
    Show {
        service: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Merge a bare or wrapped billing JSON file into the current settings
    Update {
        service: String,
        #[arg(long)]
        file: PathBuf,
        #[arg(long)]
        dry_run: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Configure a verified fixed-model Telnyx text-chat contract and three prices
    SetManaged {
        service: String,
        #[command(flatten)]
        managed: ManagedBillingArgs,
        #[arg(long)]
        probe_response: Option<PathBuf>,
        #[arg(long)]
        dry_run: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Attempt Lago synchronization once; exit 2 if any row remains unsynced
    Resync {
        service: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Check saved configuration and exact fresh rates (exit 2 when not ready)
    Verify {
        service: String,
        #[arg(long)]
        managed: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
}

#[derive(Args, Clone)]
pub struct ManagedBillingArgs {
    #[arg(long)]
    pub model: String,
    #[arg(long)]
    pub max_input_tokens: u64,
    #[arg(long)]
    pub max_output_tokens: u64,
    #[arg(long)]
    pub max_request_bytes: u64,
    #[arg(long)]
    pub max_response_bytes: u64,
    #[arg(long)]
    pub tariff_version: String,
    #[arg(long)]
    pub tariff_source: String,
    #[arg(long)]
    pub tariff_verified_at: String,
    /// Exact credits per uncached input token
    #[arg(long)]
    pub input_price: String,
    /// Exact credits per output token
    #[arg(long)]
    pub output_price: String,
    /// Exact credits per cached input token (may be zero)
    #[arg(long)]
    pub cache_read_price: String,
}

#[derive(Subcommand)]
pub enum BillingLedgerCommands {
    /// Walk a bounded range, or all chunks, and check the newest head anchor
    Verify {
        #[arg(long, value_parser = clap::value_parser!(i64).range(1..))]
        from_seq: Option<i64>,
        #[arg(long, value_parser = clap::value_parser!(i64).range(1..))]
        to_seq: Option<i64>,
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
        limit: Option<u64>,
        #[arg(long)]
        all: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
}

#[derive(Subcommand)]
pub enum BillingExecutionCommands {
    /// Latest 50 self-scoped executions; --since filters this bounded list
    List {
        #[arg(long)]
        since: Option<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Read execution metadata, optionally waiting for settled or released
    Show {
        id: String,
        #[arg(long)]
        wait: bool,
        #[arg(long, default_value_t = 120, requires = "wait", value_parser = clap::value_parser!(u64).range(0..=86400))]
        timeout_secs: u64,
        #[command(flatten)]
        auth: AuthArgs,
    },
}

#[derive(Args)]
pub struct AdminUsageArgs {
    #[arg(long, value_parser = ["24h", "7d", "30d"], conflicts_with_all = ["from", "to"])]
    pub period: Option<String>,
    #[arg(long, requires = "to")]
    pub from: Option<String>,
    #[arg(long, requires = "from")]
    pub to: Option<String>,
    #[arg(long)]
    pub service: Option<String>,
    #[arg(long)]
    pub user: Option<String>,
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u64).range(1..))]
    pub page: u64,
    #[arg(long, default_value_t = 25, value_parser = clap::value_parser!(u64).range(1..=100))]
    pub per_page: u64,
    #[command(flatten)]
    pub auth: AuthArgs,
}

#[derive(Subcommand)]
pub enum AdminCreditsCommands {
    /// Issue, inspect or revoke one-time credit grants
    Grant {
        #[command(subcommand)]
        command: CreditGrantCommands,
    },
}

#[derive(Subcommand)]
pub enum CreditGrantCommands {
    /// Issue a non-idempotent grant batch; asks for confirmation unless --yes
    Issue {
        #[arg(long)]
        user: String,
        /// Whole credits (the grant endpoint accepts integer amounts)
        #[arg(long, value_parser = clap::value_parser!(i64).range(1..=1_000_000))]
        amount: i64,
        #[arg(
            long,
            required_unless_present = "all_services",
            conflicts_with = "all_services"
        )]
        service: Vec<String>,
        #[arg(long)]
        all_services: bool,
        #[arg(long)]
        expires_at: Option<String>,
        #[arg(long)]
        reason: String,
        #[arg(long)]
        yes: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// List credit grants, optionally for a user UUID or exact email
    List {
        #[arg(long)]
        user: Option<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Revoke a grant by UUID
    Revoke {
        grant_id: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
}
