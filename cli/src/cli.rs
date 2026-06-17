use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(
    name = "nyxid",
    about = "NyxID CLI -- manage your NyxID account from the terminal",
    version
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Log in to NyxID (opens browser by default)
    Login(LoginArgs),
    /// Log out and clear stored token
    Logout(BaseUrlArgs),
    /// Register a new account
    Register(RegisterArgs),
    /// Verify email address with token
    VerifyEmail(VerifyEmailArgs),
    /// Request a password reset email
    ForgotPassword(ForgotPasswordArgs),
    /// Reset password using a reset token
    ResetPassword(ResetPasswordArgs),
    /// Show current user info
    Whoami(AuthArgs),
    /// Show account overview (services, keys, nodes)
    Status(AuthArgs),
    /// Run local CLI health checks
    Doctor(DoctorArgs),
    /// Manage user profile
    Profile {
        #[command(subcommand)]
        command: ProfileCommands,
    },
    /// Manage multi-factor authentication
    Mfa {
        #[command(subcommand)]
        command: MfaCommands,
    },
    /// Manage sessions
    Session {
        #[command(subcommand)]
        command: SessionCommands,
    },
    /// Browse the service catalog
    Catalog {
        #[command(subcommand)]
        command: CatalogCommands,
    },
    /// Manage AI services (external APIs)
    Service {
        #[command(subcommand)]
        command: ServiceCommands,
    },
    /// Manage connected provider tokens
    Provider {
        #[command(subcommand)]
        command: ProviderCommands,
    },
    /// Manage NyxID API keys
    ApiKey {
        #[command(subcommand)]
        command: ApiKeyCommands,
    },
    /// Manage organizations (shared credentials across multiple users)
    Org {
        #[command(subcommand)]
        command: OrgCommands,
    },
    /// Manage credential nodes
    Node {
        #[command(subcommand)]
        command: NodeCommands,
    },
    /// Approve device-code provisioning requests
    Device {
        #[command(subcommand)]
        command: DeviceCommands,
    },
    /// Push credential setup metadata to node operators
    #[command(name = "node-credential")]
    NodeCredential {
        #[command(subcommand)]
        command: NodeCredentialAdminCommands,
    },
    /// Proxy requests through NyxID
    Proxy {
        #[command(subcommand)]
        command: ProxyCommands,
    },
    /// Call admin-enabled anonymous public proxy endpoints
    Public {
        #[command(subcommand)]
        command: PublicCommands,
    },
    /// SSH client helper commands
    Ssh(SshCli),
    /// Set up OpenClaw integration
    Openclaw {
        #[command(subcommand)]
        command: OpenClawCommands,
    },
    /// Generate MCP configuration for AI tools
    Mcp {
        #[command(subcommand)]
        command: McpCommands,
    },
    /// Manage notification settings
    Notification {
        #[command(subcommand)]
        command: NotificationCommands,
    },
    /// Manage OAuth broker bindings (server-side credential handles)
    Oauth {
        #[command(subcommand)]
        command: OauthCommands,
    },
    /// Manage approval requests and grants
    Approval {
        #[command(subcommand)]
        command: ApprovalCommands,
    },
    /// Manage user endpoints
    Endpoint {
        #[command(subcommand)]
        command: EndpointCommands,
    },
    /// Manage external API keys/credentials
    ExternalKey {
        #[command(subcommand)]
        command: ExternalKeyCommands,
    },
    /// Manage service accounts (machine-to-machine OAuth2 client-credentials identities)
    #[command(name = "service-account")]
    ServiceAccount {
        #[command(subcommand)]
        command: ServiceAccountCommands,
    },
    /// Manage developer OAuth applications (OIDC clients for downstream apps)
    #[command(name = "developer-app")]
    DeveloperApp {
        #[command(subcommand)]
        command: DeveloperAppCommands,
    },
    /// Set up persistent AI skills for coding assistants
    AiSetup {
        #[command(subcommand)]
        command: AiSetupCommands,
    },
    /// Update the CLI and installed skills
    Update(UpdateArgs),
    /// Manage channel bot relay (Telegram/Discord/Lark/Feishu bridge to agents)
    ChannelBot {
        #[command(subcommand)]
        command: ChannelBotCommands,
    },
    /// Push device/analyzer events through the HTTP Event Gateway
    ChannelEvent {
        #[command(subcommand)]
        command: ChannelEventCommands,
    },
    /// Administrative commands (admin role required)
    Admin {
        #[command(subcommand)]
        command: AdminCommands,
    },
    /// Manage telemetry consent on this machine (see `docs/TELEMETRY.md` §3)
    Telemetry {
        #[command(subcommand)]
        command: TelemetryCommands,
    },
    /// Call ChatGPT Pro (and other browser oracles) through NyxID
    Oracle {
        #[command(subcommand)]
        command: OracleCommands,
    },
    /// Show the NyxID project repository URL
    Repo(RepoArgs),
    /// Resume a `--no-wait` remote pairing and pick up the result
    Pairing {
        #[command(subcommand)]
        command: PairingCommands,
    },
    /// Show CLI version and project links
    Info,
}

#[derive(Subcommand)]
pub enum ProviderCommands {
    /// Disconnect a personal or org-owned provider token
    Disconnect {
        /// Provider ID
        provider_id: String,
        /// Disconnect the provider token owned by this org (admin required)
        #[arg(
            long,
            value_name = "ID|SLUG|NAME",
            help = "Organization to act on (UUID, slug, or display name)"
        )]
        org: Option<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
}

#[derive(Subcommand)]
pub enum PairingCommands {
    /// Poll an existing pairing (created by `--no-wait`) until it
    /// completes or expires; print the kind-specific success summary.
    Resume {
        /// Pairing id returned on stdout by the `--no-wait` invocation.
        id: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
}

#[derive(Args)]
pub struct RepoArgs {
    /// Open the repository in the default browser
    #[arg(long)]
    pub open: bool,
}

#[derive(Subcommand)]
pub enum AdminCommands {
    /// Manage invite codes used to gate new user registration
    InviteCode {
        #[command(subcommand)]
        command: InviteCodeCommands,
    },
    /// Inspect users and manage platform roles (admin / operator / user)
    User {
        #[command(subcommand)]
        command: AdminUserCommands,
    },
}

#[derive(Subcommand)]
pub enum AdminUserCommands {
    /// List platform users (paginated)
    List {
        /// Page number (1-based)
        #[arg(long, default_value_t = 1)]
        page: u64,
        /// Results per page (max 100)
        #[arg(long, default_value_t = 50)]
        per_page: u64,
        /// Case-insensitive email substring search
        #[arg(long)]
        search: Option<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Show a single user's profile by ID
    Show {
        /// User ID (UUID)
        id: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Set a user's platform role (admin | operator | user)
    SetRole {
        /// User ID (UUID)
        id: String,
        /// New platform role
        #[arg(long, value_parser = ["admin", "operator", "user"])]
        role: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
}

#[derive(Subcommand)]
pub enum InviteCodeCommands {
    /// Create a new invite code
    Create {
        /// Maximum number of registrations this code can grant (1-1000, default 10)
        #[arg(long)]
        max_uses: Option<i32>,
        /// Optional admin note describing the intended recipient(s)
        #[arg(long)]
        note: Option<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// List all invite codes
    List {
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Deactivate an invite code by ID
    Deactivate {
        /// Invite code ID (UUID)
        id: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
}

// ---- Shared auth args ----

#[derive(Args, Clone)]
pub struct AuthArgs {
    /// NyxID base URL, e.g. https://auth.nyxid.dev (saved from login)
    #[arg(long, env = "NYXID_URL")]
    pub base_url: Option<String>,
    /// Access token (overrides saved token)
    #[arg(long)]
    pub access_token: Option<String>,
    /// Environment variable to read the access token from
    #[arg(long, default_value = "NYXID_ACCESS_TOKEN")]
    pub access_token_env: String,
    /// Agent profile name (isolates tokens and config)
    #[arg(long, env = "NYXID_PROFILE")]
    pub profile: Option<String>,
    /// Output format: table or json
    #[arg(long, default_value = "table")]
    pub output: OutputFormat,
}

impl AuthArgs {
    /// Resolve base_url: flag > env > saved from login (profile-aware)
    pub fn resolved_base_url(&self) -> anyhow::Result<String> {
        if let Some(url) = &self.base_url {
            return Ok(url.clone());
        }
        if let Some(url) = crate::auth::read_saved_base_url_for(self.profile.as_deref()) {
            return Ok(url);
        }
        anyhow::bail!(
            "No base URL configured. Run `nyxid login --base-url <URL>` first, \
             or pass --base-url, or set NYXID_URL"
        )
    }
}

#[derive(Args, Clone)]
pub struct BaseUrlArgs {
    /// NyxID base URL (saved from login)
    #[arg(long, env = "NYXID_URL")]
    pub base_url: Option<String>,
    /// Agent profile name
    #[arg(long, env = "NYXID_PROFILE")]
    pub profile: Option<String>,
}

impl BaseUrlArgs {
    pub fn resolved_base_url(&self) -> anyhow::Result<String> {
        if let Some(url) = &self.base_url {
            return Ok(url.clone());
        }
        if let Some(url) = crate::auth::read_saved_base_url_for(self.profile.as_deref()) {
            return Ok(url);
        }
        anyhow::bail!(
            "No base URL configured. Run `nyxid login --base-url <URL>` first, \
             or pass --base-url, or set NYXID_URL"
        )
    }
}

#[derive(Args, Clone)]
pub struct DoctorArgs {
    /// Emit machine-readable JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Clone)]
pub struct UpdateArgs {
    /// Only update installed skills, skip CLI binary update
    #[arg(long)]
    pub skills_only: bool,
    /// Install a specific release version, e.g. 0.4.0
    #[arg(long)]
    pub version: Option<String>,
    /// Print installed and latest versions without installing anything
    #[arg(long)]
    pub check: bool,
    /// Retarget the active nyxid symlink to the previous retained version
    #[arg(long, conflicts_with_all = ["version", "check", "from_source", "list_versions", "skills_only"])]
    pub rollback: bool,
    /// Print installed prebuilt versions without downloading anything
    #[arg(long, conflicts_with_all = ["version", "check", "from_source", "rollback", "skills_only"])]
    pub list_versions: bool,
    /// Compile and install from source with cargo install
    #[arg(long)]
    pub from_source: bool,
    /// Continue after release attestation verification fails
    #[arg(long)]
    pub insecure_skip_verify: bool,
    /// NyxID base URL for skill content (uses saved URL by default)
    #[arg(long, env = "NYXID_URL")]
    pub base_url: Option<String>,
}

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum OutputFormat {
    Table,
    Json,
}

#[derive(Subcommand)]
pub enum PublicCommands {
    /// Send a no-auth request to /public/s/{slug}/{path}
    Request {
        /// Service slug
        slug: String,
        /// Public endpoint path
        path: String,
        /// HTTP method
        #[arg(short = 'X', long, default_value = "GET")]
        method: String,
        /// Request body. Use @file to read from a file or - for stdin.
        #[arg(short = 'd', long)]
        data: Option<String>,
        /// Header to send to the downstream endpoint. Repeatable.
        #[arg(short = 'H', long = "header")]
        headers: Vec<String>,
        /// Stream response bytes to stdout
        #[arg(long)]
        stream: bool,
        #[command(flatten)]
        base: BaseUrlArgs,
    },
}

// ---- Login ----

#[derive(Args)]
pub struct LoginArgs {
    /// NyxID base URL, e.g. https://auth.nyxid.dev
    #[arg(
        long,
        env = "NYXID_URL",
        default_value = "https://nyx-api.chrono-ai.fun"
    )]
    pub base_url: String,
    /// Use email/password login instead of opening the browser
    #[arg(long)]
    pub password: bool,
    /// Email address (only used with --password)
    #[arg(long)]
    pub email: Option<String>,
    /// Agent profile name (isolates tokens)
    #[arg(long, env = "NYXID_PROFILE")]
    pub profile: Option<String>,
}

// ---- Register (C1) ----

#[derive(Args)]
pub struct RegisterArgs {
    /// NyxID base URL
    #[arg(long, env = "NYXID_URL")]
    pub base_url: String,
    /// Email address
    #[arg(long)]
    pub email: String,
    /// Display name
    #[arg(long)]
    pub name: Option<String>,
    /// Read password from this environment variable
    #[arg(long)]
    pub password_env: Option<String>,
    /// Invite code (required — request one from an admin)
    #[arg(long)]
    pub invite_code: String,
}

// ---- VerifyEmail (C2) ----

#[derive(Args)]
pub struct VerifyEmailArgs {
    /// NyxID base URL
    #[arg(long, env = "NYXID_URL")]
    pub base_url: String,
    /// Verification token from email
    #[arg(long)]
    pub token: String,
}

// ---- ForgotPassword (C3) ----

#[derive(Args)]
pub struct ForgotPasswordArgs {
    /// NyxID base URL
    #[arg(long, env = "NYXID_URL")]
    pub base_url: String,
    /// Email address
    #[arg(long)]
    pub email: String,
}

// ---- ResetPassword (C4) ----

#[derive(Args)]
pub struct ResetPasswordArgs {
    /// NyxID base URL
    #[arg(long, env = "NYXID_URL")]
    pub base_url: String,
    /// Reset token from email
    #[arg(long)]
    pub token: String,
    /// Read new password from this environment variable
    #[arg(long)]
    pub password_env: Option<String>,
}

// ---- Profile (C5, I1-I3) ----

#[derive(Subcommand)]
pub enum ProfileCommands {
    /// Update profile (name, etc.)
    Update {
        /// New display name
        #[arg(long)]
        name: Option<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Delete your account
    Delete {
        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// List OAuth consents
    Consents {
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Revoke an OAuth consent
    RevokeConsent {
        /// OAuth client ID to revoke
        client_id: String,
        /// Skip confirmation
        #[arg(long)]
        yes: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
}

// ---- MFA (C6) ----

#[derive(Subcommand)]
pub enum MfaCommands {
    /// Set up MFA — opens the wizard in your browser to render the QR code,
    /// collect the TOTP verification code, and display recovery codes once.
    Setup {
        /// Skip the browser wizard and use the scripted enrollment path.
        /// Prints the TOTP secret + provisioning URL to the terminal so you
        /// can hand them to a separate authenticator and finish via
        /// `nyxid mfa verify`. Equivalent to setting `NYXID_NO_WIZARD=1`.
        #[arg(long, alias = "no-wizard")]
        terminal: bool,
        /// Remote-pair mode: create a pairing, print the URL + code,
        /// and EXIT without polling. Pick up the result later with
        /// `nyxid pairing resume <PAIRING_ID>`. Useful for AI agents
        /// that can't hold a long-running subprocess.
        #[arg(long)]
        no_wait: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Verify a pending MFA setup with a TOTP code (scripted path).
    /// Prefer `nyxid mfa setup` — the wizard handles both halves of
    /// enrollment in one browser tab and never prints recovery codes
    /// to the terminal.
    Verify {
        /// TOTP code from authenticator app
        #[arg(long)]
        code: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Show current MFA status
    Status {
        #[command(flatten)]
        auth: AuthArgs,
    },
}

// ---- Session (C7) ----

#[derive(Subcommand)]
pub enum SessionCommands {
    /// List active sessions
    List {
        #[command(flatten)]
        auth: AuthArgs,
    },
}

// ---- Telemetry ----

/// Subcommands under `nyxid telemetry`. Canonical editor for the
/// persisted consent flag at `~/.nyxid/config.toml`. See
/// `docs/TELEMETRY.md` §3 for the full precedence ladder.
#[derive(Subcommand)]
pub enum TelemetryCommands {
    /// Opt in: persist `{enabled=true, asked=true}` to config.
    Enable,
    /// Opt out: persist `{enabled=false, asked=true}` and clear the
    /// local anon UUID so a future re-enable starts fresh.
    Disable,
    /// Print the resolved consent state and its source.
    Status,
}

// ---- Catalog ----

#[derive(Subcommand)]
pub enum CatalogCommands {
    /// List available services from the catalog
    List {
        /// Include all active services (including system services without auth)
        #[arg(long)]
        all: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Show details for a catalog entry
    Show {
        /// Service slug (e.g., llm-openai)
        slug: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// List API endpoints from a service's OpenAPI spec
    Endpoints {
        /// Service slug (e.g., llm-openai)
        slug: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
}

// ---- Service (C11-C13, I21-I23) ----

#[derive(Subcommand)]
// The `Add` variant has accreted a lot of flags as catalog adds have
// gained capabilities (org targeting, OpenAPI spec URL, WS frame
// presets, multi-connection BYO Custom App credentials, etc.). Each
// `Option<String>` adds ~24 bytes, which trips clippy's
// `large_enum_variant` heuristic. Boxing variants is the standard
// remediation but `clap` does not derive its `Args` impl for `Box<Args>`
// fields, so we'd lose `#[command(flatten)]` on `AuthArgs`. The variant
// is constructed at most once per CLI invocation, so the size delta is
// not load-bearing — accept the lint here.
#[allow(clippy::large_enum_variant)]
pub enum ServiceCommands {
    /// Add a service from catalog or custom endpoint
    Add {
        /// Catalog slug (e.g., llm-openai). Omit with --custom for a custom endpoint.
        slug: Option<String>,
        /// Add a fully custom endpoint (interactive prompts)
        #[arg(long)]
        custom: bool,
        /// Custom slug for this service (omit to auto-derive from the label/catalog slug; must be unique per user).
        #[arg(long = "slug", value_name = "SLUG")]
        custom_slug: Option<String>,
        /// Use OAuth flow for authentication
        #[arg(long)]
        oauth: bool,
        /// Use device code flow for authentication
        #[arg(long)]
        device_code: bool,
        /// Route traffic through a node
        #[arg(long)]
        via_node: Option<String>,
        /// Endpoint URL override
        #[arg(long)]
        endpoint_url: Option<String>,
        /// Label for this service
        #[arg(long)]
        label: Option<String>,
        /// Auth method: bearer, bot_bearer (Discord-style "Bot " prefix), header, query, path, basic, body (inject credential into JSON body), none (skips credential entry)
        #[arg(long)]
        auth_method: Option<String>,
        /// Auth key name (e.g. Authorization, X-API-Key, or for body auth
        /// the JSON field name like `app_secret`)
        #[arg(long)]
        auth_key_name: Option<String>,
        /// Credential value (hidden from help -- use --credential-env instead)
        #[arg(long, hide = true)]
        credential: Option<String>,
        /// Read credential from this environment variable instead of prompting
        #[arg(long)]
        credential_env: Option<String>,
        /// Read credential bytes from this file. Use for multi-field credentials
        /// like `aws_sigv4` (JSON: access_key_id/secret_access_key/region/service).
        /// Pass `-` to read from stdin.
        #[arg(long, value_name = "PATH")]
        credential_file: Option<String>,
        /// Additional OAuth scopes to request on top of the provider's defaults
        /// (repeatable, comma- or space-separated). Only used with --oauth or
        /// --device-code. The upstream provider decides whether to grant them.
        /// Example: --scope "contact:contact.base:readonly,contact:department.base:readonly"
        #[arg(long = "scope", value_name = "SCOPES")]
        scopes: Vec<String>,
        /// User-provided OAuth Custom App `client_id` for providers with
        /// `credential_mode: "user"` (Lark / Feishu / Twitter / X). Sent
        /// alongside `--oauth-client-secret` / `--oauth-client-secret-env`
        /// in the `POST /keys` body so this connection's `UserApiKey`
        /// carries its own encrypted copy of the credentials — required
        /// for per-connection refresh to keep working past the first
        /// access-token expiry. Only used with `--oauth`.
        #[arg(long, value_name = "CLIENT_ID")]
        oauth_client_id: Option<String>,
        /// Companion secret for `--oauth-client-id`. Hidden from `--help`
        /// (matches `--credential`): prefer `--oauth-client-secret-env`
        /// for scripted use so the value doesn't end up in shell history.
        #[arg(long, hide = true, value_name = "CLIENT_SECRET")]
        oauth_client_secret: Option<String>,
        /// Read the Custom App `client_secret` from this environment
        /// variable instead of prompting. Mutually substitutable with
        /// `--oauth-client-secret`. Only used with `--oauth`.
        #[arg(long, value_name = "VAR")]
        oauth_client_secret_env: Option<String>,
        /// Clone encrypted OAuth client credentials from an existing
        /// connection (by its key ID or slug) instead of re-pasting
        /// `--oauth-client-id` / `--oauth-client-secret`. Mutually
        /// exclusive with those flags. Only used with `--oauth`.
        #[arg(long, value_name = "KEY_ID_OR_SLUG")]
        copy_oauth_client_from: Option<String>,
        /// Create this key under the given org (you must be an admin of that org).
        /// Every member of the org will see the resulting service in their
        /// `nyxid service list` and can proxy through it using their own
        /// NyxID account. Omit for a personal key.
        #[arg(
            long,
            value_name = "ID|SLUG|NAME",
            help = "Organization to act on (UUID, slug, or display name)"
        )]
        org: Option<String>,
        /// Optional OpenAPI spec URL for endpoint discovery. When set, AI
        /// agents (MCP, proxy discovery) surface concrete operations parsed
        /// from this spec instead of a single generic proxy tool. Pass
        /// `--openapi-spec-url ""` on a catalog key to opt out of inheriting
        /// the catalog entry's default spec URL.
        #[arg(long, value_name = "URL")]
        openapi_spec_url: Option<String>,
        /// Apply a WebSocket auth-frame preset to the created user service.
        /// Supported value: home-assistant.
        #[arg(
            long = "ws-frame-preset",
            value_name = "NAME",
            conflicts_with = "ws_frame_clear"
        )]
        ws_frame_preset: Option<String>,
        /// Clear WebSocket auth-frame rules on the created user service.
        #[arg(long = "ws-frame-clear")]
        ws_frame_clear: bool,
        /// Force the terminal (rpassword) flow and skip the browser wizard
        /// even when a local display is available. Equivalent to setting
        /// `NYXID_NO_WIZARD=1` for a single invocation.
        #[arg(long, alias = "no-wizard")]
        terminal: bool,
        /// Remote-pair mode: create a pairing, print the URL + code,
        /// and EXIT without polling. Resume with `nyxid pairing resume
        /// <PAIRING_ID>` once the browser wizard is done.
        #[arg(long)]
        no_wait: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Add an SSH service
    AddSsh {
        /// Label for this service
        #[arg(long)]
        label: String,
        /// SSH host
        #[arg(long)]
        host: String,
        /// SSH port
        #[arg(long, default_value = "22")]
        port: u16,
        /// Enable certificate authentication
        #[arg(long, conflicts_with = "node_key")]
        cert_auth: bool,
        /// Use node-local SSH private keys instead of NyxID-issued SSH certificates
        #[arg(long)]
        node_key: bool,
        /// SSH principals (comma-separated)
        #[arg(long)]
        principals: Option<String>,
        /// Certificate TTL in minutes
        #[arg(long, default_value = "30")]
        ttl: u32,
        /// Node to route through (required for SSH)
        #[arg(long)]
        via_node: String,
        /// Create the service under an organization (UUID, slug, or display name).
        /// Members of the org will see and use the SSH service through their own NyxID account.
        #[arg(long, value_name = "ID|SLUG|NAME")]
        org: Option<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Convert an SSH service between cert, node-key, and proxy-only modes
    ConvertSsh {
        /// Service slug or ID
        slug: String,
        /// Convert to node-local SSH private-key auth
        #[arg(long, conflicts_with_all = ["to_cert", "to_proxy_only"])]
        to_node_key: bool,
        /// Convert to NyxID-issued SSH certificate auth
        #[arg(long, conflicts_with_all = ["to_node_key", "to_proxy_only"])]
        to_cert: bool,
        /// Convert to proxy-only mode
        #[arg(long, conflicts_with_all = ["to_node_key", "to_cert"])]
        to_proxy_only: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// List user's configured services
    List {
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Show service details
    Show {
        /// Service ID
        id: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Manage OAuth scopes on an existing connection (re-authorize with a new
    /// scope set). Opens the browser wizard seeded with the connection's
    /// current scopes; the upstream provider re-consents.
    Scopes {
        /// Service id or slug of the existing OAuth connection
        id: String,
        /// Desired scope set, declared up front (repeatable, comma- or
        /// space-separated). Pre-seeds the wizard picker with exactly these
        /// scopes so the whole change is driven from one command — ideal for
        /// AI/CLI use. Omit to open the picker seeded with the connection's
        /// current scopes. The upstream provider still re-consents in the
        /// browser. Example: --set "tweet.read,media.write"
        #[arg(long = "set", value_name = "SCOPES")]
        set_scopes: Vec<String>,
        /// Remote-pair mode: print the URL + code and exit without polling
        /// (resume with `nyxid pairing resume <PAIRING_ID>`).
        #[arg(long)]
        no_wait: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Delete a service
    Delete {
        /// Service ID
        id: String,
        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Update service configuration
    Update {
        /// Service ID
        id: String,
        /// New display label
        #[arg(long)]
        label: Option<String>,
        /// New endpoint URL
        #[arg(long)]
        endpoint_url: Option<String>,
        /// New OpenAPI spec URL. Pass an empty string (`--openapi-spec-url ""`)
        /// to clear the existing value.
        #[arg(long, value_name = "URL")]
        openapi_spec_url: Option<String>,
        /// New node ID for routing
        #[arg(long)]
        node_id: Option<String>,
        /// Remove node routing (direct mode)
        #[arg(long)]
        no_node: bool,
        /// Set service to active
        #[arg(long, conflicts_with = "inactive")]
        active: bool,
        /// Set service to inactive
        #[arg(long, conflicts_with = "active")]
        inactive: bool,
        /// Set or replace a default HTTP header injected on every proxied
        /// request. Format: `name=value`, optionally suffixed with
        /// `:overridable` to let a caller-supplied value win. Repeat the
        /// flag to set multiple headers. Example:
        ///   --default-header 'x-openclaw-scopes=operator.read,operator.write'
        ///   --default-header 'x-api-version=v2:overridable'
        /// When any `--default-header` flag is provided, the full list
        /// replaces the service's current defaults.
        #[arg(
            long = "default-header",
            value_name = "NAME=VALUE[:overridable]",
            conflicts_with = "clear_default_headers"
        )]
        default_header: Vec<String>,
        /// Clear all default request headers for this service.
        #[arg(long)]
        clear_default_headers: bool,
        /// Apply a WebSocket auth-frame preset. Supported value:
        /// home-assistant.
        #[arg(
            long = "ws-frame-preset",
            value_name = "NAME",
            conflicts_with = "ws_frame_clear"
        )]
        ws_frame_preset: Option<String>,
        /// Clear WebSocket auth-frame rules on this user service.
        #[arg(long = "ws-frame-clear")]
        ws_frame_clear: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Rotate external credential for a service
    RotateCredential {
        /// Service ID
        id: String,
        /// Read new credential from this environment variable
        #[arg(long)]
        credential_env: Option<String>,
        /// New credential value (hidden from help -- use --credential-env instead)
        #[arg(long, hide = true)]
        credential: Option<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Change service routing (node or direct)
    Route {
        /// Service ID
        id: String,
        /// Route through this node
        #[arg(long, conflicts_with = "direct")]
        node: Option<String>,
        /// Use direct routing (no node)
        #[arg(long, conflicts_with = "node")]
        direct: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Set OAuth client credentials for a service's provider
    Credentials {
        /// Service slug or ID
        slug: String,
        /// Read OAuth client ID from this environment variable
        #[arg(long)]
        client_id_env: Option<String>,
        /// OAuth client ID (hidden from help -- use --client-id-env instead)
        #[arg(long, hide = true)]
        client_id: Option<String>,
        /// Read OAuth client secret from this environment variable
        #[arg(long)]
        client_secret_env: Option<String>,
        /// OAuth client secret (hidden from help -- use --client-secret-env instead)
        #[arg(long, hide = true)]
        client_secret: Option<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
}

// ---- API Key ----

#[derive(Subcommand)]
pub enum ApiKeyCommands {
    /// Create a new NyxID API key
    Create {
        /// Key name
        #[arg(long)]
        name: Option<String>,
        /// Scopes (space-separated: read write proxy)
        #[arg(long)]
        scopes: Option<String>,
        /// Expiry in days (0 = no expiry)
        #[arg(long)]
        expires_in_days: Option<u32>,
        /// Allowed service IDs (comma-separated)
        #[arg(long)]
        allowed_services: Option<String>,
        /// Allowed node IDs (comma-separated)
        #[arg(long)]
        allowed_nodes: Option<String>,
        /// Allow access to all services
        #[arg(long)]
        allow_all_services: bool,
        /// Allow access to all nodes
        #[arg(long)]
        allow_all_nodes: bool,
        /// Platform label (claude-code, codex, openclaw, cursor, generic)
        #[arg(long)]
        platform: Option<String>,
        /// Callback URL for channel bot relay (where NyxID sends forwarded messages)
        #[arg(long)]
        callback_url: Option<String>,
        /// Create this key under the given org (you must be an admin of that org).
        /// The key authenticates as the org — proxy calls see org-owned services
        /// directly, and every org admin can rotate / delete it via this same CLI.
        #[arg(
            long,
            value_name = "ID|SLUG|NAME",
            help = "Organization to act on (UUID, slug, or display name)"
        )]
        org: Option<String>,
        /// Skip the browser wizard and print the new key to the terminal.
        /// The new key is shown ONCE — copy it before scrolling away.
        /// Equivalent to setting `NYXID_NO_WIZARD=1` for a single invocation.
        #[arg(long, alias = "no-wizard")]
        terminal: bool,
        /// Remote-pair mode: create a pairing, print the URL + code,
        /// and EXIT without polling. Pick up the result later with
        /// `nyxid pairing resume <PAIRING_ID>`. Useful for AI agents
        /// that can't hold a long-running subprocess.
        #[arg(long)]
        no_wait: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// List API keys
    List {
        /// List keys owned by the given org instead of your personal scope
        /// (you must be an admin of that org).
        #[arg(
            long,
            value_name = "ID|SLUG|NAME",
            help = "Organization to act on (UUID, slug, or display name)"
        )]
        org: Option<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Show key details
    Show {
        id: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Rotate a key
    Rotate {
        id: String,
        /// Skip the browser wizard and print the new key to the terminal.
        /// The new key is shown ONCE — copy it before scrolling away.
        /// Equivalent to setting `NYXID_NO_WIZARD=1` for a single invocation.
        #[arg(long, alias = "no-wizard")]
        terminal: bool,
        /// Remote-pair mode: create a pairing, print the URL + code,
        /// and EXIT without polling. Resume with `nyxid pairing resume
        /// <PAIRING_ID>` once the browser wizard is done.
        #[arg(long)]
        no_wait: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Revoke a key
    Delete {
        id: String,
        #[arg(long)]
        yes: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Update key scope
    Update {
        id: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        scopes: Option<String>,
        #[arg(long)]
        allowed_services: Option<String>,
        #[arg(long)]
        allowed_nodes: Option<String>,
        #[arg(long)]
        allow_all_services: Option<bool>,
        #[arg(long)]
        allow_all_nodes: Option<bool>,
        /// Callback URL for channel bot relay (set empty string to clear)
        #[arg(long)]
        callback_url: Option<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Bind a service to an agent key (uses the service's credential automatically)
    Bind {
        /// API key ID or name
        id: String,
        /// Service slug
        #[arg(long)]
        service: String,
        /// External credential label (auto-resolved from service if omitted)
        #[arg(long)]
        credential: Option<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
}

// ---- Org ----
//
// All org commands hit /api/v1/orgs/* and are gated by org membership
// (read) or admin role (write) on the server. The actor's auth comes from
// the standard `AuthArgs`. There is no profile-aware switching here -- the
// caller is always the actor; org credentials are resolved server-side.

#[derive(Subcommand)]
pub enum OrgCommands {
    /// Create a new organization (you become the first admin)
    Create {
        /// Display name for the organization
        #[arg(long)]
        display_name: String,
        /// Optional contact email (kept private; orgs can share emails with persons)
        #[arg(long)]
        contact_email: Option<String>,
        /// Optional avatar URL (https://...)
        #[arg(long)]
        avatar_url: Option<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// List organizations you are a member of
    List {
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Show details of an organization (must be a member)
    Show {
        /// Org ID (UUID)
        id: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Update an organization's metadata (admin only)
    Update {
        /// Org ID
        id: String,
        /// New display name
        #[arg(long)]
        display_name: Option<String>,
        /// New org slug
        #[arg(long)]
        slug: Option<String>,
        /// New avatar URL. Pass an empty string to clear.
        #[arg(long)]
        avatar_url: Option<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Delete an organization (admin only). Refuses if the org still owns shared resources.
    Delete {
        /// Org ID
        id: String,
        #[arg(long)]
        yes: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Redeem an org invite link or nonce
    Join {
        /// Invite nonce or full join URL
        nonce_or_url: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Set or clear your primary organization (proxy resolution tiebreaker)
    SetPrimary {
        /// Org ID. Omit or pass --clear to unset.
        #[arg(long)]
        org_id: Option<String>,
        /// Clear the primary org.
        #[arg(long, conflicts_with = "org_id")]
        clear: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Manage members of an organization
    Member {
        #[command(subcommand)]
        command: OrgMemberCommands,
    },
    /// Manage one-time invites for an organization
    Invite {
        #[command(subcommand)]
        command: OrgInviteCommands,
    },
    /// Manage role-level default service scopes for an organization
    RoleScope {
        #[command(subcommand)]
        command: OrgRoleScopeCommands,
    },
}

#[derive(Subcommand)]
pub enum OrgMemberCommands {
    /// List members of an organization (must be a member)
    List {
        /// Org ID
        org_id: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Add a member directly by user ID (admin only). Prefer `org invite create`.
    Add {
        /// Org ID
        org_id: String,
        /// User ID of the member to add
        #[arg(long)]
        user_id: String,
        /// Role: admin, member, viewer (default: member)
        #[arg(long, default_value = "member")]
        role: String,
        /// Scope mode: `inherit` (follow role default, new default) or
        /// `override` (use --allowed-service-ids explicitly). Omit to let the
        /// server pick: inherit when no list is given, override otherwise.
        #[arg(long)]
        scope_source: Option<String>,
        /// Comma-separated list of UserService IDs to scope this member to.
        /// Implies override when provided without an explicit --scope-source.
        #[arg(long)]
        allowed_service_ids: Option<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Update a member's role or service scope (admin only)
    Update {
        /// Org ID
        org_id: String,
        /// Member user ID
        member_id: String,
        /// New role: admin, member, viewer
        #[arg(long)]
        role: Option<String>,
        /// Scope mode: `inherit` (reset to role default) or `override`
        /// (keep / set a per-member list). When set to `inherit`, any
        /// explicit --allowed-service-ids is ignored server-side.
        #[arg(long)]
        scope_source: Option<String>,
        /// Comma-separated UserService IDs to scope this member. Pass empty to clear.
        #[arg(long)]
        allowed_service_ids: Option<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Remove a member from an organization (admin only)
    Remove {
        /// Org ID
        org_id: String,
        /// Member user ID
        member_id: String,
        #[arg(long)]
        yes: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
}

#[derive(Subcommand)]
pub enum OrgInviteCommands {
    /// Issue a new one-time invite (admin only)
    Create {
        /// Org ID
        org_id: String,
        /// Role to grant on redemption: admin, member, viewer (default: member)
        #[arg(long, default_value = "member")]
        role: String,
        /// Scope mode applied to the redeemed membership: `inherit` or
        /// `override`. Defaults to `inherit` unless --allowed-service-ids
        /// is also provided.
        #[arg(long)]
        scope_source: Option<String>,
        /// Comma-separated UserService IDs to scope the new member to.
        /// Implies override when provided without an explicit --scope-source.
        #[arg(long)]
        allowed_service_ids: Option<String>,
        /// Time-to-live in hours (default: 24)
        #[arg(long)]
        ttl_hours: Option<i64>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// List outstanding invites for an organization (admin only)
    List {
        /// Org ID
        org_id: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Cancel a pending invite (admin only)
    Cancel {
        /// Org ID
        org_id: String,
        /// Invite ID
        invite_id: String,
        #[arg(long)]
        yes: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
}

#[derive(Subcommand)]
pub enum OrgRoleScopeCommands {
    /// Show the default service scope for every org role (admin only)
    List {
        /// Org ID
        org_id: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Set the default service scope for one role (admin only).
    ///
    /// Provide either --allowed-service-ids (restrict to the listed IDs)
    /// or --full-access (clear to "no restriction"). Members in `inherit`
    /// mode pick up the new scope immediately.
    Set {
        /// Org ID
        org_id: String,
        /// Role to update: admin, member, viewer
        #[arg(long)]
        role: String,
        /// Comma-separated UserService IDs this role can access.
        /// Mutually exclusive with --full-access.
        #[arg(long, conflicts_with = "full_access")]
        allowed_service_ids: Option<String>,
        /// Grant full access (no restriction) for this role.
        #[arg(long)]
        full_access: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Remove the role scope row, reverting the role to the default
    /// (full access). Equivalent to `set --full-access`, but the row is
    /// deleted rather than stored with null.
    Clear {
        /// Org ID
        org_id: String,
        /// Role: admin, member, viewer
        #[arg(long)]
        role: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
}

// ---- Node (I4-I5 user side + agent side) ----

#[derive(Subcommand)]
pub enum NodeCommands {
    // --- User-side commands (API calls) ---
    /// List user's nodes
    List {
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Show node details
    Show {
        id: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Generate a registration token
    RegisterToken {
        /// Node name (defaults to `my-node` if neither flag nor browser
        /// wizard input is provided).
        #[arg(long)]
        name: Option<String>,
        /// Organization to act on (UUID, slug, or display name). You must be an admin of the org. Accepts the legacy --owner alias for backwards compatibility.
        #[arg(
            long,
            value_name = "ID|SLUG|NAME",
            aliases = ["owner"],
            help = "Organization to act on (UUID, slug, or display name)"
        )]
        org: Option<String>,
        /// Skip the browser wizard and print the new token to the
        /// terminal. The new token is shown ONCE — copy it before
        /// scrolling away. Equivalent to setting `NYXID_NO_WIZARD=1`
        /// for a single invocation.
        #[arg(long, alias = "no-wizard")]
        terminal: bool,
        /// Remote-pair mode: create a pairing, print the URL + code,
        /// and EXIT without polling. Resume with `nyxid pairing resume
        /// <PAIRING_ID>` once the browser wizard is done.
        #[arg(long)]
        no_wait: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Delete a node
    Delete {
        /// Node ID
        id: String,
        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Transfer a node to a person or org owner
    Transfer {
        /// Node ID or name
        id: String,
        /// Destination user or org ID
        #[arg(long, value_name = "USER_OR_ORG_ID")]
        to: String,
        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Rotate node auth token
    RotateToken {
        /// Node ID
        id: String,
        /// Skip the browser wizard and print the new auth token + signing
        /// secret to the terminal. The new values are shown ONCE — copy
        /// them before scrolling away. Equivalent to setting
        /// `NYXID_NO_WIZARD=1` for a single invocation.
        #[arg(long, alias = "no-wizard")]
        terminal: bool,
        /// Remote-pair mode: create a pairing, print the URL + code,
        /// and EXIT without polling. Resume with `nyxid pairing resume
        /// <PAIRING_ID>` once the browser wizard is done.
        #[arg(long)]
        no_wait: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },

    // --- Agent-side commands (local node operations) ---
    /// Register this node with a NyxID server
    Register {
        /// One-time registration token (nyx_nreg_...)
        #[arg(long)]
        token: String,
        /// WebSocket URL of the NyxID server
        #[arg(long)]
        url: Option<String>,
        /// Path to config directory
        #[arg(long)]
        config: Option<String>,
        /// Store secrets in the OS keychain instead of encrypted file
        #[arg(long)]
        keychain: bool,
        /// Agent profile name for multi-instance support
        #[arg(long, env = "NYXID_PROFILE")]
        profile: Option<String>,
    },
    /// Start the node agent (connect and serve)
    Start {
        /// Path to config directory
        #[arg(long)]
        config: Option<String>,
        /// Log level (trace, debug, info, warn, error)
        #[arg(long)]
        log_level: Option<String>,
        /// Agent profile name
        #[arg(long, env = "NYXID_PROFILE")]
        profile: Option<String>,
    },
    /// Show node connection status (local)
    #[command(name = "agent-status")]
    AgentStatus {
        /// Path to config directory
        #[arg(long)]
        config: Option<String>,
        /// Agent profile name
        #[arg(long, env = "NYXID_PROFILE")]
        profile: Option<String>,
    },
    /// Update the node's auth token and signing secret after a server-side rotation
    Rekey {
        /// Replacement auth token (nyx_nauth_...)
        #[arg(long)]
        auth_token: String,
        /// Replacement HMAC signing secret (64 hex chars)
        #[arg(long)]
        signing_secret: String,
        /// Path to config directory
        #[arg(long)]
        config: Option<String>,
        /// Agent profile name
        #[arg(long, env = "NYXID_PROFILE")]
        profile: Option<String>,
    },
    /// Manage local credentials
    Credentials {
        #[command(subcommand)]
        command: NodeCredentialCommands,
        /// Path to config directory
        #[arg(long)]
        config: Option<String>,
        /// Agent profile name
        #[arg(long, env = "NYXID_PROFILE")]
        profile: Option<String>,
    },
    /// Manage local SSH node-key credentials
    #[command(name = "ssh-credentials")]
    SshCredentials {
        #[command(subcommand)]
        command: NodeSshCredentialCommands,
        /// Path to config directory
        #[arg(long)]
        config: Option<String>,
        /// Agent profile name
        #[arg(long, env = "NYXID_PROFILE")]
        profile: Option<String>,
    },
    /// Migrate secret storage from file to OS keychain (or vice versa)
    Migrate {
        /// Target backend: "keychain" or "file"
        #[arg(long)]
        to: String,
        /// Path to config directory
        #[arg(long)]
        config: Option<String>,
        /// Agent profile name
        #[arg(long, env = "NYXID_PROFILE")]
        profile: Option<String>,
    },
    /// Manage OpenClaw integration (connect, status, disconnect)
    #[command(name = "openclaw")]
    NodeOpenclaw {
        #[command(subcommand)]
        command: NodeOpenClawCommands,
        /// Path to config directory
        #[arg(long)]
        config: Option<String>,
        /// Agent profile name
        #[arg(long, env = "NYXID_PROFILE")]
        profile: Option<String>,
    },
    /// Show node agent version
    #[command(name = "agent-version")]
    AgentVersion,

    /// Manage the node agent background service (install, start, stop, restart, status, logs)
    Daemon {
        #[command(subcommand)]
        command: NodeDaemonCommands,
    },

    /// Run the node agent as a Docker container
    Docker {
        #[command(subcommand)]
        command: NodeDockerCommands,
    },
}

// ---- Node credential admin commands (laptop side) ----

#[derive(Subcommand)]
pub enum NodeCredentialAdminCommands {
    /// Push credential setup metadata to a node operator
    Push {
        /// Node ID or name
        node: String,
        /// Service slug
        #[arg(long)]
        slug: String,
        /// Injection method
        #[arg(long, value_enum)]
        injection_method: PendingCredentialInjectionMethod,
        /// Header name, query parameter name, or path prefix value
        #[arg(long)]
        field_name: String,
        /// Target URL associated with this credential
        #[arg(long)]
        target_url: Option<String>,
        /// Human-readable label
        #[arg(long)]
        label: Option<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Inject a pending credential through remote credential encryption
    Inject {
        /// Node ID or name
        node: String,
        /// Existing pending credential ID to complete
        #[arg(long)]
        pending: Option<String>,
        /// Service slug
        #[arg(long, required_unless_present = "pending")]
        slug: Option<String>,
        /// Injection method
        #[arg(long, value_enum, required_unless_present = "pending")]
        injection_method: Option<PendingCredentialInjectionMethod>,
        /// Header name, query parameter name, or path prefix value
        #[arg(long, required_unless_present = "pending")]
        field_name: Option<String>,
        /// Target URL associated with this credential
        #[arg(long)]
        target_url: Option<String>,
        /// Human-readable label
        #[arg(long)]
        label: Option<String>,
        /// Organization to act on (UUID, slug, or display name)
        #[arg(long, value_name = "ID|SLUG|NAME")]
        org: Option<String>,
        /// Read the secret from this environment variable
        #[arg(long, conflicts_with = "browser")]
        secret_env: Option<String>,
        /// Open the browser accept page; the CLI process never reads the secret
        #[arg(long)]
        browser: bool,
        /// Expected node pubkey fingerprint (bare 32-char lowercase hex)
        #[arg(long, conflicts_with = "yes")]
        verify_fingerprint: Option<String>,
        /// Skip out-of-band pubkey fingerprint confirmation
        #[arg(long)]
        yes: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// List pending credential pushes for a node
    List {
        /// Node ID or name
        node: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Cancel a pending credential push
    Cancel {
        /// Node ID or name
        node: String,
        /// Pending credential ID
        pending_id: String,
        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum PendingCredentialInjectionMethod {
    Header,
    QueryParam,
    PathPrefix,
}

impl PendingCredentialInjectionMethod {
    pub fn wire_value(self) -> &'static str {
        match self {
            Self::Header => "header",
            Self::QueryParam => "query-param",
            Self::PathPrefix => "path-prefix",
        }
    }
}

// ---- Device-code commands ----

#[derive(Subcommand)]
pub enum DeviceCommands {
    /// Approve a device binding code and provision device credentials
    Approve {
        /// User code shown by the device. Dashes and spaces are optional.
        user_code: String,
        /// Approve the device under this org (admin required)
        #[arg(
            long,
            value_name = "ID|SLUG|NAME",
            help = "Organization to act on (UUID, slug, or display name)"
        )]
        org: Option<String>,
        /// Device label to store on the provisioned API key and node
        #[arg(long)]
        label: Option<String>,
        /// Service(s) to grant the device proxy access to at approve time. Repeatable. Accepts slugs or service IDs. Without this, the device gets an api_key with no service access; you'll need to grant scopes separately later.
        #[arg(long = "service", value_name = "SLUG_OR_UUID")]
        service: Vec<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Provision a no-WiFi headless device and print a nyxprov QR payload
    Onboard {
        /// Device label to store on the provisioned API key and node
        #[arg(long)]
        label: String,
        /// WiFi SSID to embed in the QR payload
        #[arg(long)]
        ssid: String,
        /// Environment variable containing the WiFi password
        #[arg(
            long = "password-env",
            value_name = "ENV_VAR",
            help = "Environment variable containing the WiFi password. A bare --password flag is intentionally unsupported so secrets do not land in shell history or process listings."
        )]
        password_env: String,
        /// Approve the device under this org (admin required)
        #[arg(
            long,
            value_name = "ID|SLUG|NAME",
            help = "Organization to act on (UUID, slug, or display name)"
        )]
        org: Option<String>,
        /// Service(s) to grant the device proxy access to at onboard time. Repeatable. Accepts slugs or service IDs.
        #[arg(long = "service", value_name = "SLUG_OR_UUID")]
        service: Vec<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Generate Ed25519 factory provisioning keys
    FactoryKey {
        /// Number of keypairs to generate
        #[arg(long, default_value_t = 1)]
        count: usize,
        /// Write output to this file instead of stdout
        #[arg(long)]
        out: Option<PathBuf>,
        /// Emit newline-delimited JSON instead of a JSON array
        #[arg(long)]
        ndjson: bool,
    },
}

// ---- Node Docker subcommands ----

#[derive(Args, Clone)]
pub struct NodeDockerArgs {
    /// Agent profile name (each profile runs as a separate container)
    #[arg(long, env = "NYXID_PROFILE")]
    pub profile: Option<String>,
}

#[derive(Subcommand)]
pub enum NodeDockerCommands {
    /// Build the node agent Docker image
    Build,
    /// Start a node agent container (mounts the profile's config directory)
    Start {
        #[command(flatten)]
        args: NodeDockerArgs,
    },
    /// Stop and remove the node agent container
    Stop {
        #[command(flatten)]
        args: NodeDockerArgs,
    },
    /// Restart the node agent container
    Restart {
        #[command(flatten)]
        args: NodeDockerArgs,
    },
    /// Show Docker container status
    Status {
        #[command(flatten)]
        args: NodeDockerArgs,
    },
    /// Tail Docker container logs
    Logs {
        #[command(flatten)]
        args: NodeDockerArgs,
        /// Follow log output (like tail -f)
        #[arg(long, short)]
        follow: bool,
    },
}

// ---- Node Daemon subcommands ----

#[derive(Args, Clone)]
pub struct NodeDaemonArgs {
    /// Path to config directory
    #[arg(long)]
    pub config: Option<String>,
    /// Agent profile name for multi-instance support
    #[arg(long, env = "NYXID_PROFILE")]
    pub profile: Option<String>,
}

#[derive(Subcommand)]
pub enum NodeDaemonCommands {
    /// Install the node agent as a background service (launchd on macOS, systemd on Linux)
    Install {
        #[command(flatten)]
        args: NodeDaemonArgs,
        /// Log level for the daemon (trace, debug, info, warn, error)
        #[arg(long)]
        log_level: Option<String>,
        /// Overwrite existing service files
        #[arg(long)]
        force: bool,
    },
    /// Uninstall the background service
    Uninstall {
        #[command(flatten)]
        args: NodeDaemonArgs,
    },
    /// Start the installed background service
    Start {
        #[command(flatten)]
        args: NodeDaemonArgs,
    },
    /// Stop the running background service
    Stop {
        #[command(flatten)]
        args: NodeDaemonArgs,
    },
    /// Restart the background service
    Restart {
        #[command(flatten)]
        args: NodeDaemonArgs,
    },
    /// Show background service status
    Status {
        #[command(flatten)]
        args: NodeDaemonArgs,
    },
    /// Show node agent logs
    Logs {
        #[command(flatten)]
        args: NodeDaemonArgs,
        /// Follow log output (like tail -f)
        #[arg(long, short)]
        follow: bool,
        /// Number of recent lines to show
        #[arg(long, short, default_value = "50")]
        lines: usize,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn daemon_subcommands_accept_config_after_subcommand() {
        let cli = Cli::try_parse_from(["nyxid", "node", "daemon", "install", "--config", "/tmp"])
            .expect("daemon install should accept --config after the subcommand");

        match cli.command {
            Commands::Node {
                command:
                    NodeCommands::Daemon {
                        command:
                            NodeDaemonCommands::Install {
                                args: NodeDaemonArgs { config, .. },
                                ..
                            },
                    },
            } => assert_eq!(config.as_deref(), Some("/tmp")),
            _ => panic!("unexpected parse result"),
        }
    }

    #[test]
    fn device_approve_accepts_code_org_and_label() {
        let cli = Cli::try_parse_from([
            "nyxid",
            "device",
            "approve",
            "ABCD-EFGH-JKLM",
            "--org",
            "team-ai",
            "--label",
            "Hallway camera",
        ])
        .expect("device approve should parse");

        match cli.command {
            Commands::Device {
                command:
                    DeviceCommands::Approve {
                        user_code,
                        org,
                        label,
                        service,
                        ..
                    },
            } => {
                assert_eq!(user_code, "ABCD-EFGH-JKLM");
                assert_eq!(org.as_deref(), Some("team-ai"));
                assert_eq!(label.as_deref(), Some("Hallway camera"));
                assert!(service.is_empty());
            }
            _ => panic!("unexpected parse result"),
        }
    }

    #[test]
    fn device_approve_accepts_repeatable_service_flags() {
        let single = Cli::try_parse_from([
            "nyxid",
            "device",
            "approve",
            "ABCD-EFGH-JKLM",
            "--service",
            "llm-openai",
        ])
        .expect("device approve should parse one service");

        match single.command {
            Commands::Device {
                command: DeviceCommands::Approve { service, .. },
            } => assert_eq!(service, vec!["llm-openai".to_string()]),
            _ => panic!("unexpected parse result"),
        }

        let cli = Cli::try_parse_from([
            "nyxid",
            "device",
            "approve",
            "ABCD-EFGH-JKLM",
            "--service",
            "llm-openai",
            "--service",
            "550e8400-e29b-41d4-a716-446655440000",
        ])
        .expect("device approve should parse repeatable services");

        match cli.command {
            Commands::Device {
                command: DeviceCommands::Approve { service, .. },
            } => assert_eq!(
                service,
                vec![
                    "llm-openai".to_string(),
                    "550e8400-e29b-41d4-a716-446655440000".to_string()
                ]
            ),
            _ => panic!("unexpected parse result"),
        }
    }

    #[test]
    fn device_onboard_requires_password_env_and_accepts_services() {
        let cli = Cli::try_parse_from([
            "nyxid",
            "device",
            "onboard",
            "--label",
            "Kitchen Camera",
            "--ssid",
            "MyNetwork",
            "--password-env",
            "WIFI_PASSWORD",
            "--org",
            "team-ai",
            "--service",
            "llm-openai",
            "--service",
            "550e8400-e29b-41d4-a716-446655440000",
        ])
        .expect("device onboard should parse");

        match cli.command {
            Commands::Device {
                command:
                    DeviceCommands::Onboard {
                        label,
                        ssid,
                        password_env,
                        org,
                        service,
                        ..
                    },
            } => {
                assert_eq!(label, "Kitchen Camera");
                assert_eq!(ssid, "MyNetwork");
                assert_eq!(password_env, "WIFI_PASSWORD");
                assert_eq!(org.as_deref(), Some("team-ai"));
                assert_eq!(
                    service,
                    vec![
                        "llm-openai".to_string(),
                        "550e8400-e29b-41d4-a716-446655440000".to_string()
                    ]
                );
            }
            _ => panic!("unexpected parse result"),
        }

        assert!(
            Cli::try_parse_from([
                "nyxid",
                "device",
                "onboard",
                "--label",
                "Kitchen Camera",
                "--ssid",
                "MyNetwork",
            ])
            .is_err()
        );
    }

    #[test]
    fn device_onboard_does_not_accept_bare_password_flag() {
        assert!(
            Cli::try_parse_from([
                "nyxid",
                "device",
                "onboard",
                "--label",
                "Kitchen Camera",
                "--ssid",
                "MyNetwork",
                "--password",
                "hunter22",
            ])
            .is_err()
        );
    }

    #[test]
    fn device_factory_key_accepts_count_out_and_ndjson() {
        let cli = Cli::try_parse_from([
            "nyxid",
            "device",
            "factory-key",
            "--count",
            "2",
            "--out",
            "keys.ndjson",
            "--ndjson",
        ])
        .expect("device factory-key should parse");

        match cli.command {
            Commands::Device {
                command: DeviceCommands::FactoryKey { count, out, ndjson },
            } => {
                assert_eq!(count, 2);
                assert_eq!(out.as_deref(), Some(std::path::Path::new("keys.ndjson")));
                assert!(ndjson);
            }
            _ => panic!("unexpected parse result"),
        }
    }

    #[test]
    fn service_add_accepts_terminal_flag() {
        for flag in ["--terminal", "--no-wizard"] {
            let cli = Cli::try_parse_from(["nyxid", "service", "add", "llm-openai", flag])
                .unwrap_or_else(|e| panic!("service add should accept {flag}: {e}"));

            match cli.command {
                Commands::Service {
                    command: ServiceCommands::Add { terminal, .. },
                } => assert!(terminal, "{flag} should set terminal=true"),
                _ => panic!("unexpected parse result for {flag}"),
            }
        }

        let cli = Cli::try_parse_from(["nyxid", "service", "add", "llm-openai"])
            .expect("service add without flag should parse");
        match cli.command {
            Commands::Service {
                command: ServiceCommands::Add { terminal, .. },
            } => assert!(!terminal, "terminal should default to false"),
            _ => panic!("unexpected parse result"),
        }
    }

    #[test]
    fn service_add_accepts_custom_slug_flag_without_shadowing_catalog_slug() {
        let cli = Cli::try_parse_from([
            "nyxid",
            "service",
            "add",
            "llm-openai",
            "--slug",
            "my-custom-service",
        ])
        .expect("service add should parse both catalog slug and custom slug flag");

        match cli.command {
            Commands::Service {
                command:
                    ServiceCommands::Add {
                        slug, custom_slug, ..
                    },
            } => {
                assert_eq!(slug.as_deref(), Some("llm-openai"));
                assert_eq!(custom_slug.as_deref(), Some("my-custom-service"));
            }
            _ => panic!("unexpected parse result"),
        }
    }

    #[test]
    fn provider_disconnect_accepts_org_flag() {
        let cli = Cli::try_parse_from([
            "nyxid",
            "provider",
            "disconnect",
            "provider-1",
            "--org",
            "org-1",
        ])
        .expect("provider disconnect should accept --org");

        match cli.command {
            Commands::Provider {
                command:
                    ProviderCommands::Disconnect {
                        provider_id, org, ..
                    },
            } => {
                assert_eq!(provider_id, "provider-1");
                assert_eq!(org.as_deref(), Some("org-1"));
            }
            _ => panic!("unexpected parse result"),
        }
    }

    #[test]
    fn node_register_token_accepts_org_flag() {
        let cli = Cli::try_parse_from([
            "nyxid",
            "node",
            "register-token",
            "--name",
            "edge-node",
            "--org",
            "11111111-2222-3333-4444-555555555555",
        ])
        .expect("node register-token should accept --org");

        match cli.command {
            Commands::Node {
                command: NodeCommands::RegisterToken { name, org, .. },
            } => {
                assert_eq!(name.as_deref(), Some("edge-node"));
                assert_eq!(org.as_deref(), Some("11111111-2222-3333-4444-555555555555"));
            }
            _ => panic!("unexpected parse result"),
        }
    }

    #[test]
    fn node_register_token_accepts_owner_alias_for_org_flag() {
        let cli = Cli::try_parse_from([
            "nyxid",
            "node",
            "register-token",
            "--name",
            "edge-node",
            "--owner",
            "chrono-ai",
        ])
        .expect("node register-token should accept --owner alias");

        match cli.command {
            Commands::Node {
                command: NodeCommands::RegisterToken { name, org, .. },
            } => {
                assert_eq!(name.as_deref(), Some("edge-node"));
                assert_eq!(org.as_deref(), Some("chrono-ai"));
            }
            _ => panic!("unexpected parse result"),
        }
    }

    #[test]
    fn node_transfer_accepts_to_and_yes_flags() {
        let cli = Cli::try_parse_from([
            "nyxid",
            "node",
            "transfer",
            "edge-node",
            "--to",
            "11111111-2222-3333-4444-555555555555",
            "--yes",
        ])
        .expect("node transfer should accept --to and --yes");

        match cli.command {
            Commands::Node {
                command: NodeCommands::Transfer { id, to, yes, .. },
            } => {
                assert_eq!(id, "edge-node");
                assert_eq!(to, "11111111-2222-3333-4444-555555555555");
                assert!(yes);
            }
            _ => panic!("unexpected parse result"),
        }
    }

    #[test]
    fn node_credential_push_accepts_metadata_flags() {
        let cli = Cli::try_parse_from([
            "nyxid",
            "node-credential",
            "push",
            "edge-node",
            "--slug",
            "openclaw",
            "--injection-method",
            "query-param",
            "--field-name",
            "api_key",
            "--target-url",
            "https://gateway.example.com/v1",
            "--label",
            "prod",
        ])
        .expect("node-credential push should parse");

        match cli.command {
            Commands::NodeCredential {
                command:
                    NodeCredentialAdminCommands::Push {
                        node,
                        slug,
                        injection_method,
                        field_name,
                        target_url,
                        label,
                        ..
                    },
            } => {
                assert_eq!(node, "edge-node");
                assert_eq!(slug, "openclaw");
                assert_eq!(
                    injection_method,
                    PendingCredentialInjectionMethod::QueryParam
                );
                assert_eq!(field_name, "api_key");
                assert_eq!(
                    target_url.as_deref(),
                    Some("https://gateway.example.com/v1")
                );
                assert_eq!(label.as_deref(), Some("prod"));
            }
            _ => panic!("unexpected parse result"),
        }
    }

    #[test]
    fn node_credential_inject_accepts_modes_and_flags() {
        let cli = Cli::try_parse_from([
            "nyxid",
            "node-credential",
            "inject",
            "edge-node",
            "--slug",
            "openclaw",
            "--injection-method",
            "header",
            "--field-name",
            "Authorization",
            "--target-url",
            "https://gateway.example.com/v1",
            "--label",
            "prod",
            "--org",
            "chrono-ai",
            "--secret-env",
            "OPENCLAW_KEY",
            "--verify-fingerprint",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ])
        .expect("node-credential inject should parse");

        match cli.command {
            Commands::NodeCredential {
                command:
                    NodeCredentialAdminCommands::Inject {
                        node,
                        pending,
                        slug,
                        injection_method,
                        field_name,
                        target_url,
                        label,
                        org,
                        secret_env,
                        browser,
                        verify_fingerprint,
                        yes,
                        ..
                    },
            } => {
                assert_eq!(node, "edge-node");
                assert!(pending.is_none());
                assert_eq!(slug.as_deref(), Some("openclaw"));
                assert_eq!(
                    injection_method,
                    Some(PendingCredentialInjectionMethod::Header)
                );
                assert_eq!(field_name.as_deref(), Some("Authorization"));
                assert_eq!(
                    target_url.as_deref(),
                    Some("https://gateway.example.com/v1")
                );
                assert_eq!(label.as_deref(), Some("prod"));
                assert_eq!(org.as_deref(), Some("chrono-ai"));
                assert_eq!(secret_env.as_deref(), Some("OPENCLAW_KEY"));
                assert!(!browser);
                assert_eq!(
                    verify_fingerprint.as_deref(),
                    Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
                );
                assert!(!yes);
            }
            _ => panic!("unexpected parse result"),
        }
    }

    #[test]
    fn inject_requires_node_with_pending() {
        let err = match Cli::try_parse_from([
            "nyxid",
            "node-credential",
            "inject",
            "--pending",
            "pending-1",
        ]) {
            Ok(_) => panic!("--pending should still require the node positional"),
            Err(err) => err,
        };

        assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
    }

    #[test]
    fn node_credentials_accept_accepts_value_env() {
        let cli = Cli::try_parse_from([
            "nyxid",
            "node",
            "credentials",
            "accept",
            "openclaw",
            "--value-env",
            "OPENCLAW_KEY",
        ])
        .expect("node credentials accept should parse");

        match cli.command {
            Commands::Node {
                command:
                    NodeCommands::Credentials {
                        command: NodeCredentialCommands::Accept { slug, value_env },
                        ..
                    },
            } => {
                assert_eq!(slug, "openclaw");
                assert_eq!(value_env.as_deref(), Some("OPENCLAW_KEY"));
            }
            _ => panic!("unexpected parse result"),
        }
    }

    #[test]
    fn service_add_oauth_accepts_custom_slug_flag() {
        let cli = Cli::try_parse_from([
            "nyxid", "service", "add", "api-lark", "--oauth", "--slug", "my-lark",
        ])
        .expect("service add oauth should parse custom slug");

        match cli.command {
            Commands::Service {
                command:
                    ServiceCommands::Add {
                        slug,
                        custom_slug,
                        oauth,
                        ..
                    },
            } => {
                assert_eq!(slug.as_deref(), Some("api-lark"));
                assert_eq!(custom_slug.as_deref(), Some("my-lark"));
                assert!(oauth);
            }
            _ => panic!("unexpected parse result"),
        }
    }

    #[test]
    fn service_add_device_code_accepts_custom_slug_flag() {
        let cli = Cli::try_parse_from([
            "nyxid",
            "service",
            "add",
            "llm-openai",
            "--device-code",
            "--slug",
            "openai-team-a",
        ])
        .expect("service add device-code should parse custom slug");

        match cli.command {
            Commands::Service {
                command:
                    ServiceCommands::Add {
                        slug,
                        custom_slug,
                        device_code,
                        ..
                    },
            } => {
                assert_eq!(slug.as_deref(), Some("llm-openai"));
                assert_eq!(custom_slug.as_deref(), Some("openai-team-a"));
                assert!(device_code);
            }
            _ => panic!("unexpected parse result"),
        }
    }

    #[test]
    fn service_add_accepts_ws_frame_preset() {
        let cli = Cli::try_parse_from([
            "nyxid",
            "service",
            "add",
            "--custom",
            "--ws-frame-preset",
            "home-assistant",
        ])
        .expect("service add should parse websocket frame preset");

        match cli.command {
            Commands::Service {
                command:
                    ServiceCommands::Add {
                        ws_frame_preset,
                        ws_frame_clear,
                        ..
                    },
            } => {
                assert_eq!(ws_frame_preset.as_deref(), Some("home-assistant"));
                assert!(!ws_frame_clear);
            }
            _ => panic!("unexpected parse result"),
        }
    }

    #[test]
    fn service_update_accepts_ws_frame_clear() {
        let cli = Cli::try_parse_from([
            "nyxid",
            "service",
            "update",
            "service-1",
            "--ws-frame-clear",
        ])
        .expect("service update should parse websocket frame clear");

        match cli.command {
            Commands::Service {
                command:
                    ServiceCommands::Update {
                        id, ws_frame_clear, ..
                    },
            } => {
                assert_eq!(id, "service-1");
                assert!(ws_frame_clear);
            }
            _ => panic!("unexpected parse result"),
        }
    }

    #[test]
    fn service_ws_frame_preset_and_clear_conflict() {
        let err = match Cli::try_parse_from([
            "nyxid",
            "service",
            "update",
            "service-1",
            "--ws-frame-preset",
            "home-assistant",
            "--ws-frame-clear",
        ]) {
            Ok(_) => panic!("preset and clear should be mutually exclusive"),
            Err(err) => err,
        };

        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
    }
}

// ---- Node Credential subcommands ----

#[derive(Subcommand)]
pub enum NodeCredentialCommands {
    /// Add a credential for a service (prompts for the secret value securely)
    Add {
        /// Service slug (e.g., "openai", "github-api")
        #[arg(long)]
        service: String,
        /// Target URL for this service (e.g., "https://api.openai.com/v1")
        #[arg(long)]
        url: Option<String>,
        /// Header name to inject (e.g., "Authorization")
        #[arg(long)]
        header: Option<String>,
        /// Query parameter name to inject (e.g., "api_key")
        #[arg(long)]
        query_param: Option<String>,
        /// How to format the prompted secret before storing it
        #[arg(long, value_enum, default_value_t = CredentialSecretFormat::Raw)]
        secret_format: CredentialSecretFormat,
        /// Inline secret value (skips interactive prompt; NOT recommended)
        #[arg(long, hide = true)]
        value: Option<String>,
    },
    /// Add an OAuth credential for a service (runs device code or authorization code flow)
    AddOauth {
        /// Service slug (e.g., "api-twitter", "llm-openai")
        #[arg(long)]
        service: String,
        /// Fetch OAuth config from NyxID catalog
        #[arg(long)]
        from_catalog: bool,
        /// OAuth client ID
        #[arg(long)]
        client_id: Option<String>,
        /// OAuth client secret
        #[arg(long)]
        client_secret: Option<String>,
        /// OAuth authorization URL
        #[arg(long)]
        authorization_url: Option<String>,
        /// OAuth token URL
        #[arg(long)]
        token_url: Option<String>,
        /// Device code URL
        #[arg(long)]
        device_code_url: Option<String>,
        /// Replace the catalog's default scopes entirely with the space-separated
        /// list provided here. Legacy power-user escape hatch; prefer `--scope`
        /// to add extras on top of the defaults.
        #[arg(long)]
        scopes: Option<String>,
        /// Additional OAuth scopes to append to the provider's defaults
        /// (repeatable, comma- or space-separated). Unlike `--scopes`, this is
        /// additive and mirrors `nyxid service add --scope`.
        #[arg(long = "scope", value_name = "SCOPES")]
        additional_scopes: Vec<String>,
        /// Target URL for this service
        #[arg(long)]
        url: Option<String>,
        /// NyxID API base URL
        #[arg(long)]
        api_url: Option<String>,
        /// NyxID access token
        #[arg(long)]
        access_token: Option<String>,
    },
    /// Auto-setup credentials for a service (fetches requirements from catalog)
    Setup {
        /// Service slug (e.g., "llm-openai", "api-twitter")
        #[arg(long)]
        service: String,
        /// Additional OAuth scopes to append to the catalog defaults
        /// (repeatable, comma- or space-separated). Only used when the service
        /// requires OAuth or device code; ignored for API-key credentials.
        #[arg(long = "scope", value_name = "SCOPES")]
        additional_scopes: Vec<String>,
        /// NyxID API base URL
        #[arg(long)]
        api_url: Option<String>,
        /// NyxID access token
        #[arg(long)]
        access_token: Option<String>,
    },
    /// List configured credentials
    List,
    /// List pushed credentials awaiting local acceptance
    Pending,
    /// Accept a pushed credential and store the secret locally
    Accept {
        /// Service slug to accept
        slug: String,
        /// Read the secret value from this environment variable
        #[arg(long)]
        value_env: Option<String>,
    },
    /// Decline a pushed credential without changing local credentials
    Decline {
        /// Service slug to decline
        slug: String,
        /// Optional reason to send to NyxID
        #[arg(long)]
        reason: Option<String>,
    },
    /// Remove a credential for a service
    Remove {
        /// Service slug to remove
        #[arg(long)]
        service: String,
    },
}

#[derive(Subcommand)]
pub enum NodeSshCredentialCommands {
    /// Add a per-principal SSH private key
    Add {
        /// Service slug
        #[arg(long)]
        service: String,
        /// SSH principal this key authenticates as
        #[arg(long)]
        principal: String,
        /// Private key PEM file
        #[arg(long)]
        key_file: PathBuf,
        /// Target host reached by this node
        #[arg(long)]
        host: String,
        /// Target SSH port
        #[arg(long, default_value = "22")]
        port: u16,
        /// Environment variable containing the private-key passphrase
        #[arg(long)]
        passphrase_env: Option<String>,
        /// Do not pin the target SSH host key
        #[arg(long)]
        no_pin_host_key: bool,
        /// SSH KEX algorithm allowlist (repeatable or comma-separated)
        #[arg(long, value_delimiter = ',')]
        kex: Vec<String>,
        /// SSH host-key algorithm allowlist
        #[arg(long = "host-key", value_delimiter = ',')]
        host_key: Vec<String>,
        /// SSH cipher allowlist
        #[arg(long, value_delimiter = ',')]
        cipher: Vec<String>,
        /// SSH MAC allowlist
        #[arg(long, value_delimiter = ',')]
        mac: Vec<String>,
    },
    /// Set or reset SSH algorithm allowlists for one credential
    SetAlgos {
        /// Service slug
        #[arg(long)]
        service: String,
        /// SSH principal
        #[arg(long)]
        principal: String,
        /// SSH KEX algorithm allowlist (repeatable or comma-separated)
        #[arg(long, value_delimiter = ',')]
        kex: Vec<String>,
        /// SSH host-key algorithm allowlist
        #[arg(long = "host-key", value_delimiter = ',')]
        host_key: Vec<String>,
        /// SSH cipher allowlist
        #[arg(long, value_delimiter = ',')]
        cipher: Vec<String>,
        /// SSH MAC allowlist
        #[arg(long, value_delimiter = ',')]
        mac: Vec<String>,
        /// Reset the SSH KEX allowlist to russh defaults
        #[arg(long)]
        reset_kex: bool,
        /// Reset the SSH host-key allowlist to russh defaults
        #[arg(long)]
        reset_host_key: bool,
        /// Reset the SSH cipher allowlist to russh defaults
        #[arg(long)]
        reset_cipher: bool,
        /// Reset the SSH MAC allowlist to russh defaults
        #[arg(long)]
        reset_mac: bool,
        /// Reset all SSH algorithm allowlists to russh defaults
        #[arg(long)]
        reset_all: bool,
    },
    /// List configured SSH private keys
    List {
        /// Filter by service slug
        #[arg(long)]
        service: Option<String>,
    },
    /// Show one configured SSH private key entry
    Show {
        /// Service slug
        #[arg(long)]
        service: String,
        /// SSH principal
        #[arg(long)]
        principal: String,
    },
    /// Remove one configured SSH private key entry
    Remove {
        /// Service slug
        #[arg(long)]
        service: String,
        /// SSH principal
        #[arg(long)]
        principal: String,
    },
    /// Test a configured SSH private key with a no-op command
    Test {
        /// Service slug
        #[arg(long)]
        service: String,
        /// SSH principal
        #[arg(long)]
        principal: String,
    },
    /// Prune stale SSH private keys
    Prune {
        /// Remove entries for services that are no longer in node-key mode
        #[arg(long)]
        stale: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
}

// ---- Node OpenClaw subcommands ----

#[derive(Subcommand)]
pub enum NodeOpenClawCommands {
    /// Connect to an OpenClaw gateway
    Connect {
        /// OpenClaw gateway URL (e.g., http://localhost:18789)
        #[arg(long)]
        url: String,
        /// OpenClaw gateway bearer token
        #[arg(long)]
        token: Option<String>,
        /// NyxID API base URL
        #[arg(long)]
        api_url: Option<String>,
        /// NyxID access token
        #[arg(long)]
        access_token: Option<String>,
    },
    /// Show OpenClaw connection status
    Status,
    /// Disconnect from OpenClaw
    Disconnect,
}

// ---- Credential secret format ----

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum CredentialSecretFormat {
    /// Store the secret exactly as entered.
    Raw,
    /// Prefix the secret with "Bearer ".
    Bearer,
    /// Base64-encode "username:password" and prefix it with "Basic ".
    Basic,
}

// ---- Proxy (C8-C10) ----

#[derive(Subcommand)]
pub enum ProxyCommands {
    /// List proxyable services (service discovery)
    Discover {
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Send a request through the NyxID proxy
    Request {
        /// Service slug or UUID
        service: String,
        /// Request path (e.g., v1/chat/completions)
        #[arg(default_value = "")]
        path: String,
        /// HTTP method
        #[arg(long, short, default_value = "GET")]
        method: String,
        /// Request body (JSON string, or @file to read from file, or - for stdin)
        #[arg(long, short)]
        data: Option<String>,
        /// Extra headers (repeatable, format: Key:Value)
        #[arg(long = "header", short = 'H')]
        headers: Vec<String>,
        /// Stream the response (for SSE, video, audio, large files)
        #[arg(long)]
        stream: bool,
        /// Use service ID instead of slug
        #[arg(long)]
        by_id: bool,
        /// Use a specific UserService ID instead of auto-resolution.
        /// Get the ID from `nyxid service list --output json`.
        /// When both personal and org credentials exist for the same
        /// slug, this lets you choose which one the proxy uses.
        #[arg(long, value_name = "USER_SERVICE_ID")]
        via_service: Option<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
}

// ---- SSH (I27-I28) ----

#[derive(Args)]
pub struct SshCli {
    #[command(subcommand)]
    pub command: SshCommand,
}

#[derive(Subcommand)]
pub enum SshCommand {
    /// Issue a short-lived SSH certificate
    IssueCert {
        /// Service ID, slug, or name
        service_id: String,
        #[arg(long)]
        public_key_file: PathBuf,
        #[arg(long)]
        principal: String,
        #[arg(long)]
        certificate_file: PathBuf,
        #[arg(long)]
        ca_public_key_file: Option<PathBuf>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Open an SSH-over-WebSocket tunnel (ProxyCommand)
    Proxy {
        /// Service ID, slug, or name
        service_id: String,
        #[arg(long, default_value_t = false)]
        issue_certificate: bool,
        #[arg(long)]
        public_key_file: Option<PathBuf>,
        #[arg(long)]
        principal: Option<String>,
        #[arg(long)]
        certificate_file: Option<PathBuf>,
        #[arg(long)]
        ca_public_key_file: Option<PathBuf>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Print an OpenSSH config stanza
    Config {
        #[arg(long)]
        host_alias: String,
        #[arg(long)]
        base_url: String,
        #[arg(long)]
        service_id: String,
        #[arg(long)]
        principal: String,
        #[arg(long)]
        identity_file: PathBuf,
        #[arg(long)]
        certificate_file: PathBuf,
        #[arg(long, default_value = "NYXID_ACCESS_TOKEN")]
        access_token_env: String,
        #[arg(long)]
        ca_public_key_file: Option<PathBuf>,
    },
    /// Execute a command on a remote host via SSH
    Exec {
        /// Service ID, slug, or name
        service_id: String,
        /// SSH principal (username)
        #[arg(long)]
        principal: Option<String>,
        /// Command to execute
        #[arg(trailing_var_arg = true, required = true)]
        command: Vec<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Open an interactive SSH terminal
    Terminal {
        /// Service ID, slug, or name
        service_id: String,
        /// SSH principal (username)
        #[arg(long)]
        principal: Option<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
}

// ---- OpenClaw ----

#[derive(Subcommand)]
pub enum OpenClawCommands {
    /// Interactive OpenClaw setup
    Setup {
        /// OpenClaw gateway URL
        #[arg(long)]
        url: Option<String>,
        /// Read bearer token from this environment variable
        #[arg(long)]
        token_env: Option<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
}

// ---- MCP ----

#[derive(Subcommand)]
pub enum McpCommands {
    /// Generate MCP configuration for AI tools
    Config {
        /// Target tool: cursor, claude-code, vscode, generic
        #[arg(long, default_value = "generic")]
        tool: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
}

// ---- Notification (I11-I14) ----

#[derive(Subcommand)]
pub enum NotificationCommands {
    /// Show current notification settings
    Settings {
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Update notification settings
    Update {
        /// Enable/disable approval email notifications
        #[arg(long)]
        approval_email: Option<bool>,
        /// Enable/disable approval push notifications
        #[arg(long)]
        approval_push: Option<bool>,
        /// Enable/disable approval Telegram notifications
        #[arg(long)]
        approval_telegram: Option<bool>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Link a Telegram account
    TelegramLink {
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Disconnect Telegram account
    TelegramDisconnect {
        #[command(flatten)]
        auth: AuthArgs,
    },
}

// ---- Approval (I15-I20) ----

#[derive(Subcommand)]
pub enum ApprovalCommands {
    /// List approval requests
    List {
        /// List approval history scoped to the given org instead of your
        /// personal scope. You must be an admin of that org.
        #[arg(
            long,
            value_name = "ID|SLUG|NAME",
            help = "Organization to act on (UUID, slug, or display name)"
        )]
        org: Option<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Show approval request details
    Show {
        /// Request ID
        id: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Approve a request
    Approve {
        /// Request ID
        id: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Deny a request
    Deny {
        /// Request ID
        id: String,
        /// Reason for denial
        #[arg(long)]
        reason: Option<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// List approval grants
    Grants {
        /// List grants owned by the given org instead of your personal scope.
        /// You must be an admin of that org. Org-policy approvals create
        /// grants under the org's user_id, so this is the only way for org
        /// admins to see / manage them.
        #[arg(
            long,
            value_name = "ID|SLUG|NAME",
            help = "Organization to act on (UUID, slug, or display name)"
        )]
        org: Option<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Revoke an approval grant
    RevokeGrant {
        /// Grant ID
        id: String,
        /// Revoke a grant owned by the given org. You must be an admin
        /// of that org.
        #[arg(
            long,
            value_name = "ID|SLUG|NAME",
            help = "Organization to act on (UUID, slug, or display name)"
        )]
        org: Option<String>,
        /// Skip confirmation
        #[arg(long)]
        yes: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Enable approval protection globally (requires notification channel)
    Enable {
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Disable approval protection globally
    Disable {
        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// List per-service approval configurations
    ServiceConfigs {
        /// List configs for the given org instead of your personal scope.
        /// You must be an admin of that org.
        #[arg(
            long,
            value_name = "ID|SLUG|NAME",
            help = "Organization to act on (UUID, slug, or display name)"
        )]
        org: Option<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Set approval configuration for a service
    SetConfig {
        /// UserService ID (from `nyxid service list`) or catalog
        /// DownstreamService ID. Use the UserService ID for custom
        /// services that have no catalog backing — that's the only way
        /// to target their policy. Catalog-backed user services accept
        /// either and collapse to the same policy.
        id: String,
        /// Require approval for this service
        #[arg(long)]
        require_approval: Option<bool>,
        /// Approval mode: "per_request" (every call needs approval) or "grant" (approval creates a time-based grant)
        #[arg(long)]
        approval_mode: Option<String>,
        /// Effect for operations that match no rule: "auto_allow" (allow
        /// silently), "require_approval", or "deny". Defaults to auto_allow
        /// when rules are present, so add explicit rules plus a stricter
        /// default to turn a service into an allow-list.
        #[arg(long, value_name = "EFFECT")]
        default_effect: Option<String>,
        /// Granular approval rule (repeatable; first match wins). Format is
        /// 'key=value;key=value' with keys effect, methods, path, verbs, mode.
        /// Only effect is commonly needed; methods/path default to any/'*'.
        /// effect=auto_allow|require_approval|deny, verbs=read|write|destructive
        /// (comma-separated), methods=GET,POST,... (comma-separated; also EXEC
        /// or TUNNEL for SSH), path=<glob>, mode=per_request|grant.
        /// Example: --rule 'effect=deny;methods=DELETE' --rule 'verbs=write;mode=grant'
        #[arg(long = "rule", value_name = "SPEC")]
        rules: Vec<String>,
        /// Remove all granular rules from this service, reverting to the plain
        /// require-approval/mode policy. Cannot be combined with --rule.
        #[arg(long, conflicts_with = "rules")]
        clear_rules: bool,
        /// Set the policy on the given org's behalf instead of your personal
        /// scope. You must be an admin of that org. The org's policy is
        /// authoritative for org-shared services -- it overrides any
        /// personal policy each member may have set, and notifications
        /// fan out to every active org admin.
        #[arg(
            long,
            value_name = "ID|SLUG|NAME",
            help = "Organization to act on (UUID, slug, or display name)"
        )]
        org: Option<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
}

// ---- Endpoint (I24) ----

#[derive(Subcommand)]
pub enum EndpointCommands {
    /// List user endpoints
    List {
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Update an endpoint
    Update {
        /// Endpoint ID
        id: String,
        /// New URL
        #[arg(long)]
        url: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Delete an endpoint
    Delete {
        /// Endpoint ID
        id: String,
        /// Skip confirmation
        #[arg(long)]
        yes: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
}

// ---- ExternalKey (I25-I26) ----

#[derive(Subcommand)]
pub enum ExternalKeyCommands {
    /// List external API keys/credentials
    List {
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Rotate an external credential
    Rotate {
        /// External key ID
        id: String,
        /// Read new credential from this environment variable
        #[arg(long)]
        credential_env: Option<String>,
        /// New credential value (hidden from help -- use --credential-env instead)
        #[arg(long, hide = true)]
        credential: Option<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Delete an external credential
    Delete {
        /// External key ID
        id: String,
        /// Skip confirmation
        #[arg(long)]
        yes: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Register a Google Cloud service-account JSON key as a proxy credential.
    ///
    /// Service accounts give unattended, never-reauth access to Google
    /// Cloud APIs (BigQuery, Cloud Billing) -- unlike user OAuth, which
    /// Google rejects with `invalid_rapt` after its ~16h session lapses.
    AddGcpServiceAccount {
        /// Path to the Google service-account JSON key file
        #[arg(long, value_name = "PATH")]
        key_file: PathBuf,
        /// Display label (defaults to the service-account client_email)
        #[arg(long)]
        label: Option<String>,
        /// OAuth scope(s) to mint, space-separated
        /// (default: https://www.googleapis.com/auth/cloud-platform)
        #[arg(long)]
        scopes: Option<String>,
        /// Service slug(s) to (re)bind to this credential, e.g.
        /// `google-bigquery`. Repeatable.
        #[arg(long = "service", value_name = "SLUG")]
        services: Vec<String>,
        /// Create this credential under the given org (you must be an admin).
        #[arg(long, value_name = "ID|SLUG|NAME")]
        org: Option<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
}

// ---- Service Account (SUP-030) ----

#[derive(Subcommand)]
pub enum ServiceAccountCommands {
    /// Create a service account (machine identity for `grant_type=client_credentials`)
    Create {
        /// Human-readable name for this service account
        #[arg(long)]
        name: String,
        /// Space-separated OAuth scopes the SA may request (e.g. "openid profile")
        #[arg(long)]
        scopes: String,
        /// Optional description
        #[arg(long)]
        description: Option<String>,
        /// Override the default rate limit (tokens per second). Admin-only.
        #[arg(long)]
        rate_limit_override: Option<u64>,
        /// Comma-separated role IDs to assign to this SA
        #[arg(long)]
        role_ids: Option<String>,
        /// Create under the given org (you must be an admin of that org).
        /// Omit to create a global SA (requires global admin).
        #[arg(
            long,
            value_name = "ID|SLUG|NAME",
            help = "Organization to act on (UUID, slug, or display name)"
        )]
        org: Option<String>,
        /// Skip the browser wizard and print the new client_secret to the terminal.
        /// The new secret is shown ONCE — copy it before scrolling away.
        /// Equivalent to setting `NYXID_NO_WIZARD=1` for a single invocation.
        #[arg(long, alias = "no-wizard")]
        terminal: bool,
        /// Remote-pair mode: create a pairing, print the URL + code,
        /// and EXIT without polling. Pick up the result later with
        /// `nyxid pairing resume <PAIRING_ID>`. Useful for AI agents
        /// that can't hold a long-running subprocess.
        #[arg(long)]
        no_wait: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// List service accounts
    List {
        /// Scope the listing to a single org (requires admin of that org).
        /// Omit to list the global set (requires global admin).
        #[arg(
            long,
            value_name = "ID|SLUG|NAME",
            help = "Organization to act on (UUID, slug, or display name)"
        )]
        org: Option<String>,
        /// Optional search filter (matches name / client_id)
        #[arg(long)]
        search: Option<String>,
        /// Page number (1-based)
        #[arg(long, default_value_t = 1)]
        page: u64,
        /// Results per page (max 100)
        #[arg(long, default_value_t = 50)]
        per_page: u64,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Show service account details
    Show {
        /// Service account ID
        id: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Update service account metadata
    Update {
        /// Service account ID
        id: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        description: Option<String>,
        #[arg(long)]
        scopes: Option<String>,
        /// Comma-separated role IDs
        #[arg(long)]
        role_ids: Option<String>,
        /// Enable (`true`) or disable (`false`) the service account
        #[arg(long)]
        is_active: Option<bool>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Delete a service account (soft-delete + token revocation)
    Delete {
        /// Service account ID
        id: String,
        #[arg(long)]
        yes: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Rotate the service account's client secret (revokes existing tokens)
    RotateSecret {
        /// Service account ID
        id: String,
        /// Skip the browser wizard and print the new client_secret to the terminal.
        /// The new secret is shown ONCE — copy it before scrolling away.
        /// Equivalent to setting `NYXID_NO_WIZARD=1` for a single invocation.
        #[arg(long, alias = "no-wizard")]
        terminal: bool,
        /// Remote-pair mode: create a pairing, print the URL + code,
        /// and EXIT without polling. Resume with `nyxid pairing resume
        /// <PAIRING_ID>` once the browser wizard is done.
        #[arg(long)]
        no_wait: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Revoke all active tokens for a service account
    RevokeTokens {
        /// Service account ID
        id: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
}

// ---- Developer App (SUP-030) ----

#[derive(Subcommand)]
pub enum DeveloperAppCommands {
    /// Create a developer OAuth client (OIDC app for downstream integrations)
    Create {
        /// Display name for the OAuth client
        #[arg(long)]
        name: String,
        /// Redirect URI. Repeat `--redirect-uri` to register multiple.
        #[arg(long = "redirect-uri", value_name = "URI")]
        redirect_uris: Vec<String>,
        /// Client type: `public` or `confidential` (default: `public`)
        #[arg(long)]
        client_type: Option<String>,
        /// Space-separated OIDC scopes this client may request
        /// (e.g. "openid profile email")
        #[arg(long)]
        allowed_scopes: Option<String>,
        /// Space-separated delegation scopes (empty string disables token exchange)
        #[arg(long)]
        delegation_scopes: Option<String>,
        #[arg(long = "broker-capability")]
        broker_capability: Option<bool>,
        /// Create under the given org (you must be an admin of that org).
        /// Omit to create a personal developer app.
        #[arg(
            long,
            value_name = "ID|SLUG|NAME",
            help = "Organization to act on (UUID, slug, or display name)"
        )]
        org: Option<String>,
        /// Skip the browser wizard and print the new client_secret to the terminal.
        /// Only relevant for `--client-type confidential`; public clients have no
        /// secret and never enter the wizard. Equivalent to `NYXID_NO_WIZARD=1`.
        #[arg(long, alias = "no-wizard")]
        terminal: bool,
        /// Remote-pair mode: create a pairing, print the URL + code,
        /// and EXIT without polling. Resume with `nyxid pairing resume
        /// <PAIRING_ID>` once the browser wizard is done.
        #[arg(long)]
        no_wait: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// List developer OAuth clients
    List {
        /// Scope the listing to a single org (requires admin of that org).
        /// Omit to list the caller's personal developer apps.
        #[arg(
            long,
            value_name = "ID|SLUG|NAME",
            help = "Organization to act on (UUID, slug, or display name)"
        )]
        org: Option<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Show developer OAuth client details
    Show {
        /// OAuth client ID
        id: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Update developer OAuth client metadata
    Update {
        /// OAuth client ID
        id: String,
        #[arg(long)]
        name: Option<String>,
        /// Redirect URI. Repeat `--redirect-uri` to replace with multiple URIs.
        #[arg(long = "redirect-uri", value_name = "URI")]
        redirect_uris: Vec<String>,
        /// Space-separated OIDC scopes. Empty string canonicalizes to "openid".
        #[arg(long)]
        allowed_scopes: Option<String>,
        /// Space-separated delegation scopes (empty string disables token exchange)
        #[arg(long)]
        delegation_scopes: Option<String>,
        #[arg(long = "broker-capability")]
        broker_capability: Option<bool>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Delete a developer OAuth client (soft-delete)
    Delete {
        /// OAuth client ID
        id: String,
        #[arg(long)]
        yes: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Rotate a confidential client's secret
    RotateSecret {
        /// OAuth client ID
        id: String,
        /// Skip the browser wizard and print the new client_secret to the terminal.
        /// The new secret is shown ONCE — copy it before scrolling away.
        /// Equivalent to setting `NYXID_NO_WIZARD=1` for a single invocation.
        #[arg(long, alias = "no-wizard")]
        terminal: bool,
        /// Remote-pair mode: create a pairing, print the URL + code,
        /// and EXIT without polling. Resume with `nyxid pairing resume
        /// <PAIRING_ID>` once the browser wizard is done.
        #[arg(long)]
        no_wait: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
}

#[derive(Subcommand)]
pub enum OauthCommands {
    /// Manage broker bindings owned by the current user
    Bindings {
        #[command(subcommand)]
        command: BindingCommands,
    },
}

#[derive(Subcommand)]
pub enum BindingCommands {
    /// List all active broker bindings owned by the current user
    List {
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Show details for a specific binding by its hash (prefix match accepted)
    Show {
        /// Binding hash (full SHA-256 hex, or any unique prefix of 8+ chars)
        id_or_hash: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Revoke a binding by its hash (prefix match accepted)
    Revoke {
        /// Binding hash (full SHA-256 hex, or any unique prefix of 8+ chars)
        id_or_hash: String,
        /// Skip confirmation prompt
        #[arg(long, short = 'y')]
        yes: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
}

// ---- AI Setup ----

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum AiToolTarget {
    /// Claude Code (Anthropic CLI)
    ClaudeCode,
    /// Cursor editor
    Cursor,
    /// OpenAI Codex CLI
    Codex,
    /// OpenClaw AI gateway
    Openclaw,
    /// Generic / other AI tool
    Generic,
}

impl std::fmt::Display for AiToolTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ClaudeCode => write!(f, "claude-code"),
            Self::Cursor => write!(f, "cursor"),
            Self::Codex => write!(f, "codex"),
            Self::Openclaw => write!(f, "openclaw"),
            Self::Generic => write!(f, "generic"),
        }
    }
}

#[derive(Subcommand)]
pub enum AiSetupCommands {
    /// Install persistent NyxID skills for an AI tool
    Install {
        /// Target AI tool
        #[arg(long, value_enum)]
        tool: AiToolTarget,
        /// NyxID base URL (uses saved URL from login if omitted)
        #[arg(long, env = "NYXID_URL")]
        base_url: Option<String>,
    },
    /// Update installed skills to the latest version
    Update {
        /// Update only this tool (updates all installed if omitted)
        #[arg(long, value_enum)]
        tool: Option<AiToolTarget>,
        /// NyxID base URL
        #[arg(long, env = "NYXID_URL")]
        base_url: Option<String>,
    },
    /// Show which AI skills are currently installed
    Status,
}

// ---- Channel Bot Relay ----

#[derive(Subcommand)]
pub enum ChannelBotCommands {
    /// Register a new messaging platform bot
    Register {
        /// Platform: telegram, discord, lark, feishu, slack
        #[arg(long)]
        platform: String,
        /// Bot token (hidden from help -- use --token-env instead).
        /// Slack: pass the `xoxb-` bot user OAuth token.
        #[arg(long, hide = true)]
        bot_token: Option<String>,
        /// Read bot token from this environment variable
        #[arg(long)]
        token_env: Option<String>,
        /// Label for this bot
        #[arg(long)]
        label: String,
        /// Platform app ID (required for Lark/Feishu)
        #[arg(long)]
        app_id: Option<String>,
        /// App secret (hidden from help -- use --app-secret-env instead).
        /// Lark/Feishu: app secret. Slack: app signing secret.
        #[arg(long, hide = true)]
        app_secret: Option<String>,
        /// Read app secret from this environment variable
        #[arg(long)]
        app_secret_env: Option<String>,
        /// Lark/Feishu verification token. Falls back to
        /// NYXID_LARK_VERIFICATION_TOKEN when omitted.
        #[arg(long)]
        verification_token: Option<String>,
        /// Optional Lark/Feishu Encrypt Key. Falls back to
        /// NYXID_LARK_ENCRYPT_KEY when omitted.
        #[arg(long)]
        encrypt_key: Option<String>,
        /// Platform public key (required for Discord)
        #[arg(long)]
        public_key: Option<String>,
        /// Create this bot under the given org (you must be an admin of
        /// that org). Omit for a personal bot.
        #[arg(
            long,
            value_name = "ID|SLUG|NAME",
            help = "Organization to act on (UUID, slug, or display name)"
        )]
        org: Option<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Update an existing bot's label or platform verification material
    Update {
        /// Bot ID
        id: String,
        /// New label
        #[arg(long)]
        label: Option<String>,
        /// Lark/Feishu verification token. Falls back to
        /// NYXID_LARK_VERIFICATION_TOKEN when omitted.
        #[arg(long)]
        verification_token: Option<String>,
        /// Optional Lark/Feishu Encrypt Key. Falls back to
        /// NYXID_LARK_ENCRYPT_KEY when omitted.
        #[arg(long)]
        encrypt_key: Option<String>,
        /// Lark/Feishu App ID
        #[arg(long)]
        app_id: Option<String>,
        /// Lark/Feishu App Secret or Slack signing secret
        #[arg(long, hide = true)]
        app_secret: Option<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// List registered bots
    List {
        /// List bots owned by the given org (admin-only). Omit for
        /// personal bots.
        #[arg(
            long,
            value_name = "ID|SLUG|NAME",
            help = "Organization to act on (UUID, slug, or display name)"
        )]
        org: Option<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Show bot details
    Show {
        /// Bot ID
        id: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Delete a bot
    Delete {
        /// Bot ID
        id: String,
        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Verify bot token and re-register webhook
    Verify {
        /// Bot ID
        id: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Manage conversation routes
    Route {
        #[command(subcommand)]
        command: ChannelRouteCommands,
    },
}

#[derive(Subcommand)]
pub enum ChannelRouteCommands {
    /// Create a conversation route
    Create {
        /// Bot ID to route from
        #[arg(long)]
        bot_id: String,
        /// NyxID API key ID to relay messages to
        #[arg(long)]
        agent_key_id: String,
        /// Platform conversation ID (omit for catch-all default route)
        #[arg(long)]
        conversation_id: Option<String>,
        /// Conversation type: private, group, channel (default: private)
        #[arg(long)]
        conversation_type: Option<String>,
        /// Platform sender ID filter
        #[arg(long)]
        sender_id: Option<String>,
        /// Mark as the default agent for unmatched conversations
        #[arg(long)]
        default_agent: bool,
        /// Create this route under the given org (you must be an admin
        /// of that org). The bot and agent key must also belong to the
        /// same org. Omit for a personal route.
        #[arg(
            long,
            value_name = "ID|SLUG|NAME",
            help = "Organization to act on (UUID, slug, or display name)"
        )]
        org: Option<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// List conversation routes
    List {
        /// Filter by bot ID
        #[arg(long)]
        bot_id: Option<String>,
        /// List routes owned by the given org (admin-only). Omit for
        /// personal routes.
        #[arg(
            long,
            value_name = "ID|SLUG|NAME",
            help = "Organization to act on (UUID, slug, or display name)"
        )]
        org: Option<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Update a conversation route
    Update {
        /// Route ID
        id: String,
        /// New agent API key ID
        #[arg(long)]
        agent_key_id: Option<String>,
        /// Set as default agent
        #[arg(long)]
        default_agent: Option<bool>,
        /// Set route active/inactive
        #[arg(long)]
        active: Option<bool>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Delete a conversation route
    Delete {
        /// Route ID
        id: String,
        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
}

// ---- HTTP Event Gateway ----

#[derive(Subcommand)]
pub enum ChannelEventCommands {
    /// Push a device/analyzer event to an agent via the HTTP Event Gateway
    ///
    /// Requires a NyxID API key (`nyxid_ag_...`) that is bound to the target
    /// conversation as the agent key. Session tokens from `nyxid login` are
    /// rejected by the endpoint — the CLI does not fall back to them.
    Push {
        /// Target conversation ID (path parameter)
        #[arg(long)]
        conversation_id: String,
        /// Logical source of the event (e.g. "camera-analyzer")
        #[arg(long)]
        source: String,
        /// Event type (e.g. "person_detected")
        #[arg(long = "type")]
        event_type: String,
        /// Event ID — UUID; auto-generated if omitted
        #[arg(long)]
        event_id: Option<String>,
        /// Event timestamp (RFC 3339); defaults to now
        #[arg(long)]
        timestamp: Option<String>,
        /// Inline payload JSON (e.g. '{"room":"living_room"}')
        #[arg(long, conflicts_with = "payload_file")]
        payload_json: Option<String>,
        /// Read payload JSON from file (`-` for stdin)
        #[arg(long, conflicts_with = "payload_json")]
        payload_file: Option<String>,
        /// Inline metadata JSON (e.g. '{"analyzer_version":"1.0"}')
        #[arg(long)]
        metadata_json: Option<String>,
        /// API key (`nyxid_ag_...`); prompts if neither flag is provided
        #[arg(long, hide = true)]
        api_key: Option<String>,
        /// Read API key from this environment variable
        #[arg(long)]
        api_key_env: Option<String>,
        #[command(flatten)]
        base: BaseUrlArgs,
        /// Output format: table or json
        #[arg(long, default_value = "table")]
        output: OutputFormat,
    },
    /// Manage device channels (HTTP Event Gateway conversations, NyxID#221).
    ///
    /// Device channels are NOT backed by a bot — they exist purely as
    /// conversation rows on the channel relay pipeline so the gateway has
    /// somewhere to address events to.
    Channel {
        #[command(subcommand)]
        command: ChannelEventChannelCommands,
    },
}

#[derive(Subcommand)]
pub enum ChannelEventChannelCommands {
    /// Create a device channel (platform = "device").
    Create {
        /// Logical device channel ID (the name devices POST to).
        #[arg(long)]
        conversation_id: String,
        /// Agent API key ID that handles events for this channel.
        #[arg(long)]
        agent_key_id: String,
        /// Conversation type label surfaced to the agent. Defaults to "device".
        #[arg(long)]
        conversation_type: Option<String>,
        /// Create under the given org (caller must be an admin of that org).
        #[arg(
            long,
            value_name = "ID|SLUG|NAME",
            help = "Organization to act on (UUID, slug, or display name)"
        )]
        org: Option<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// List device channels (platform = "device").
    List {
        /// List channels owned by the given org (admin-only).
        #[arg(
            long,
            value_name = "ID|SLUG|NAME",
            help = "Organization to act on (UUID, slug, or display name)"
        )]
        org: Option<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Delete a device channel by conversation ID.
    Delete {
        /// Conversation ID (the `_id` returned by `create`, not the logical
        /// channel name).
        id: String,
        /// Skip confirmation prompt.
        #[arg(long)]
        yes: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
}

// ---- Oracle (browser LLM relay) ----

#[derive(Subcommand)]
pub enum OracleCommands {
    /// Ask a one-shot question and wait for the answer
    Ask {
        /// Pool slug or id to route through (e.g. chatgpt-pro)
        pool: String,
        /// The prompt. Omit to read from --file or stdin.
        prompt: Option<String>,
        /// Read the prompt from a file (use `-` for stdin)
        #[arg(long, value_name = "PATH")]
        file: Option<String>,
        /// Attach a PDF (uploaded by the worker on the first turn)
        #[arg(long, value_name = "PATH")]
        pdf: Option<String>,
        /// Model hint forwarded to the worker (defaults to the pool's)
        #[arg(long)]
        model: Option<String>,
        /// ChatGPT Project URL for this prompt (overrides the pool default)
        #[arg(long)]
        project_url: Option<String>,
        /// Free-form tag recorded on the task
        #[arg(long)]
        tag: Option<String>,
        /// Continue an existing conversation by id (multi-turn)
        #[arg(long)]
        conversation: Option<String>,
        /// Start a multi-turn conversation and print its id
        #[arg(long, conflicts_with = "conversation")]
        new_conversation: bool,
        /// Submitter-scoped idempotency key (safe blind retries)
        #[arg(long)]
        client_ref: Option<String>,
        /// Max seconds to wait for the answer before giving up
        #[arg(long, default_value_t = 3600)]
        wait: u64,
        /// Submit and print the task id without waiting
        #[arg(long)]
        no_wait: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Fetch a task's current state (or its answer if done)
    Result {
        /// Task id returned by `oracle ask --no-wait`
        task_id: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Cancel a queued or in-flight task
    Cancel {
        /// Task id
        task_id: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Show a pool's queue and active workers
    Status {
        /// Pool slug or id
        pool: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Attach an existing ChatGPT conversation URL and import its transcript
    Attach {
        /// Pool slug or id to route through (e.g. chatgpt-pro)
        pool: String,
        /// Existing ChatGPT conversation URL (`https://chatgpt.com/c/...`)
        url: String,
        /// Free-form tag recorded on the session and scrape task
        #[arg(long)]
        tag: Option<String>,
        /// Max seconds to wait for the transcript import before giving up
        #[arg(long, default_value_t = 120)]
        wait: u64,
        /// Submit and print the conversation/task ids without waiting
        #[arg(long)]
        no_wait: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Extract readable main content from any web page
    Extract {
        /// Pool slug or id to route through (e.g. browser-worker)
        pool: String,
        /// URL to extract (`http://` or `https://`)
        url: String,
        /// Model label echoed by the worker result (defaults to the pool's)
        #[arg(long)]
        model: Option<String>,
        /// Max seconds to wait for extraction before giving up
        #[arg(long, default_value_t = 180)]
        wait: u64,
        /// Submit and print the task id without waiting
        #[arg(long)]
        no_wait: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Manage oracle pools (capacity = your browser tabs)
    Pool {
        #[command(subcommand)]
        command: OraclePoolCommands,
    },
    /// List your multi-turn conversations
    Sessions {
        /// Filter to a single pool (slug or id)
        #[arg(long)]
        pool: Option<String>,
        /// Max sessions to list
        #[arg(long, default_value_t = 50)]
        limit: i64,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Show a conversation transcript
    Session {
        /// Conversation id (conv_...)
        conversation_id: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Close a conversation (blocks further turns)
    CloseSession {
        /// Conversation id (conv_...)
        conversation_id: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
}

#[derive(Subcommand)]
pub enum OraclePoolCommands {
    /// Create a pool and print its one-time worker token
    Create {
        /// URL-safe slug (e.g. chatgpt-pro)
        slug: String,
        /// Human-readable name
        #[arg(long)]
        name: String,
        /// Description
        #[arg(long)]
        description: Option<String>,
        /// Who may submit: private (default) | org | platform
        #[arg(long, value_parser = ["private", "org", "platform"])]
        visibility: Option<String>,
        /// ChatGPT Project URL workers must stay pinned to
        #[arg(long)]
        project_url: Option<String>,
        /// Default model hint recorded on tasks
        #[arg(long)]
        model: Option<String>,
        /// Allow this pool to drive worker browsers to extract arbitrary URLs
        #[arg(long)]
        allow_extract: bool,
        /// Max tasks dispatched at once (across all worker tabs)
        #[arg(long)]
        max_workers: Option<u32>,
        /// Max queued tasks before submits are rejected
        #[arg(long)]
        max_queue: Option<u32>,
        /// Per-submitter cap on in-flight tasks
        #[arg(long)]
        per_user_inflight: Option<u32>,
        /// Dispatch lease seconds (heartbeats refresh it)
        #[arg(long)]
        task_timeout: Option<u64>,
        /// Create under this org (you must be an org admin)
        #[arg(
            long,
            value_name = "ID|SLUG|NAME",
            help = "Organization to act on (UUID, slug, or display name)"
        )]
        org: Option<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// List pools you can see
    List {
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Show one pool
    Show {
        /// Pool slug or id
        pool: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Update a pool's settings
    Update {
        /// Pool slug or id
        pool: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        description: Option<String>,
        #[arg(long, value_parser = ["private", "org", "platform"])]
        visibility: Option<String>,
        #[arg(long)]
        project_url: Option<String>,
        #[arg(long)]
        model: Option<String>,
        /// Enable or disable browser URL extraction for this pool
        #[arg(long, action = clap::ArgAction::Set)]
        allow_extract: Option<bool>,
        #[arg(long)]
        max_workers: Option<u32>,
        #[arg(long)]
        max_queue: Option<u32>,
        #[arg(long)]
        per_user_inflight: Option<u32>,
        #[arg(long)]
        task_timeout: Option<u64>,
        /// Activate or deactivate the pool
        #[arg(long)]
        active: Option<bool>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Rotate the worker token (invalidates all paired tabs)
    RotateToken {
        /// Pool slug or id
        pool: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
}
