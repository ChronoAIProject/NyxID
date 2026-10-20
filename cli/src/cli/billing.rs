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
        /// Catalog service slug or UUID
        #[arg(long)]
        service: Option<String>,
        /// Include retired historical rate rows
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
        /// Catalog service slug or UUID
        service: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Merge a bare or wrapped billing JSON file into the current settings
    Update {
        /// Catalog service slug or UUID
        service: String,
        /// Billing JSON patch file (bare or wrapped in billing)
        #[arg(long)]
        file: PathBuf,
        /// Preview the merged settings and diff without writing
        #[arg(long)]
        dry_run: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Configure a verified fixed-model Telnyx text-chat contract and three prices
    SetManaged {
        /// Catalog service slug or UUID
        service: String,
        #[command(flatten)]
        managed: ManagedBillingArgs,
        /// Validate a saved provider probe response JSON file
        #[arg(long)]
        probe_response: Option<PathBuf>,
        /// Preview the merged settings and diff without writing
        #[arg(long)]
        dry_run: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Attempt Lago synchronization once; exit 2 if any row remains unsynced
    Resync {
        /// Catalog service slug or UUID
        service: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Check saved configuration and exact fresh rates (exit 2 when not ready)
    Verify {
        /// Catalog service slug or UUID
        service: String,
        /// Check the fixed-model managed offering and published operation
        #[arg(long)]
        managed: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
}

#[derive(Args, Clone)]
pub struct ManagedBillingArgs {
    /// Exact verified Telnyx model ID
    #[arg(long)]
    pub model: String,
    /// Maximum total input tokens allowed by the contract
    #[arg(long)]
    pub max_input_tokens: u64,
    /// Maximum output tokens, below the context ceiling
    #[arg(long)]
    pub max_output_tokens: u64,
    /// Maximum upstream request size in bytes
    #[arg(long)]
    pub max_request_bytes: u64,
    /// Maximum upstream response size in bytes
    #[arg(long)]
    pub max_response_bytes: u64,
    /// Operator-assigned verified tariff version
    #[arg(long)]
    pub tariff_version: String,
    /// HTTPS Telnyx tariff evidence URL
    #[arg(long)]
    pub tariff_source: String,
    /// Tariff verification time in RFC3339 (at most 30 days old)
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
        /// First ledger sequence to verify (inclusive)
        #[arg(long, value_parser = clap::value_parser!(i64).range(1..))]
        from_seq: Option<i64>,
        /// Last ledger sequence to verify (inclusive)
        #[arg(long, value_parser = clap::value_parser!(i64).range(1..))]
        to_seq: Option<i64>,
        /// Maximum entries in each verification chunk
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
        limit: Option<u64>,
        /// Follow every verification chunk to the current head
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
        /// RFC3339 lower creation bound within the latest 50 executions
        #[arg(long)]
        since: Option<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Read execution metadata, optionally waiting for settled or released
    Show {
        /// Self-scoped managed execution UUID
        id: String,
        /// Poll until settled or released; exit 2 on timeout
        #[arg(long)]
        wait: bool,
        /// Maximum wait in seconds (0 checks once)
        #[arg(long, default_value_t = 120, requires = "wait", value_parser = clap::value_parser!(u64).range(0..=86400))]
        timeout_secs: u64,
        #[command(flatten)]
        auth: AuthArgs,
    },
}

#[derive(Args)]
pub struct AdminUsageArgs {
    /// Usage window ending now: 24h, 7d or 30d
    #[arg(long, value_parser = ["24h", "7d", "30d"], conflicts_with_all = ["from", "to"])]
    pub period: Option<String>,
    /// Inclusive RFC3339 usage window start
    #[arg(long, requires = "to")]
    pub from: Option<String>,
    /// Exclusive RFC3339 usage window end (at most 31 days)
    #[arg(long, requires = "from")]
    pub to: Option<String>,
    /// Catalog service slug or UUID
    #[arg(long)]
    pub service: Option<String>,
    /// User UUID or exact email address
    #[arg(long)]
    pub user: Option<String>,
    /// Page number, starting at 1
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u64).range(1..))]
    pub page: u64,
    /// Maximum rows per page
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
        /// User UUID or exact email address
        #[arg(long)]
        user: String,
        /// Whole credits (the grant endpoint accepts integer amounts)
        #[arg(long, value_parser = clap::value_parser!(i64).range(1..=1_000_000))]
        amount: i64,
        /// Catalog service slug or UUID (repeatable)
        #[arg(
            long,
            required_unless_present = "all_services",
            conflicts_with = "all_services"
        )]
        service: Vec<String>,
        /// Make this grant usable with every service
        #[arg(long)]
        all_services: bool,
        /// Optional grant expiry in RFC3339
        #[arg(long)]
        expires_at: Option<String>,
        /// Audit reason for issuing credits
        #[arg(long)]
        reason: String,
        /// Issue without an interactive confirmation
        #[arg(long)]
        yes: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// List credit grants, optionally for a user UUID or exact email
    List {
        /// User UUID or exact email address
        #[arg(long)]
        user: Option<String>,
        /// Page number, starting at 1
        #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u64).range(1..))]
        page: u64,
        /// Grants per page, from 1 through 500
        #[arg(long, default_value_t = 50, value_parser = clap::value_parser!(u64).range(1..=500))]
        per_page: u64,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Revoke a grant by UUID
    Revoke {
        /// Credit grant UUID to revoke
        grant_id: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
}
