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
    /// Proxy requests through NyxID
    Proxy {
        #[command(subcommand)]
        command: ProxyCommands,
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
    /// Show the NyxID project repository URL
    Repo(RepoArgs),
    /// Show CLI version and project links
    Info,
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
pub struct UpdateArgs {
    /// Only update installed skills, skip CLI binary update
    #[arg(long)]
    pub skills_only: bool,
    /// NyxID base URL for skill content (uses saved URL by default)
    #[arg(long, env = "NYXID_URL")]
    pub base_url: Option<String>,
}

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum OutputFormat {
    Table,
    Json,
}

// ---- Login ----

#[derive(Args)]
pub struct LoginArgs {
    /// NyxID base URL, e.g. https://auth.nyxid.dev
    #[arg(long, env = "NYXID_URL")]
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
    /// Set up MFA (displays QR code URL and secret)
    Setup {
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Verify MFA setup with a TOTP code
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
pub enum ServiceCommands {
    /// Add a service from catalog or custom endpoint
    Add {
        /// Catalog slug (e.g., llm-openai). Omit with --custom for a custom endpoint.
        slug: Option<String>,
        /// Add a fully custom endpoint (interactive prompts)
        #[arg(long)]
        custom: bool,
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
        /// Auth method: bearer, bot_bearer (Discord-style "Bot " prefix), header, query, path, basic, body (inject credential into JSON body), none
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
        /// Additional OAuth scopes to request on top of the provider's defaults
        /// (repeatable, comma- or space-separated). Only used with --oauth or
        /// --device-code. The upstream provider decides whether to grant them.
        /// Example: --scope "contact:contact.base:readonly,contact:department.base:readonly"
        #[arg(long = "scope", value_name = "SCOPES")]
        scopes: Vec<String>,
        /// Create this key under the given org (you must be an admin of that org).
        /// Every member of the org will see the resulting service in their
        /// `nyxid service list` and can proxy through it using their own
        /// NyxID account. Omit for a personal key.
        #[arg(long, value_name = "ORG_ID")]
        org: Option<String>,
        /// Optional OpenAPI spec URL for endpoint discovery. When set, AI
        /// agents (MCP, proxy discovery) surface concrete operations parsed
        /// from this spec instead of a single generic proxy tool. Pass
        /// `--openapi-spec-url ""` on a catalog key to opt out of inheriting
        /// the catalog entry's default spec URL.
        #[arg(long, value_name = "URL")]
        openapi_spec_url: Option<String>,
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
        #[arg(long)]
        cert_auth: bool,
        /// SSH principals (comma-separated)
        #[arg(long)]
        principals: Option<String>,
        /// Certificate TTL in minutes
        #[arg(long, default_value = "30")]
        ttl: u32,
        /// Node to route through (required for SSH)
        #[arg(long)]
        via_node: String,
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
        #[arg(long, value_name = "ORG_ID")]
        org: Option<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// List API keys
    List {
        /// List keys owned by the given org instead of your personal scope
        /// (you must be an admin of that org).
        #[arg(long, value_name = "ORG_ID")]
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
        /// Comma-separated list of UserService IDs to scope this member to.
        /// Omit for full access to all org services.
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
        /// Comma-separated UserService IDs to scope the new member to.
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
    /// Rotate node auth token
    RotateToken {
        /// Node ID
        id: String,
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
    /// Remove a credential for a service
    Remove {
        /// Service slug to remove
        #[arg(long)]
        service: String,
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
        principal: String,
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
        #[arg(long, value_name = "ORG_ID")]
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
        #[arg(long, value_name = "ORG_ID")]
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
        #[arg(long, value_name = "ORG_ID")]
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
        #[arg(long, value_name = "ORG_ID")]
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
        /// Set the policy on the given org's behalf instead of your personal
        /// scope. You must be an admin of that org. The org's policy is
        /// authoritative for org-shared services -- it overrides any
        /// personal policy each member may have set, and notifications
        /// fan out to every active org admin.
        #[arg(long, value_name = "ORG_ID")]
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
}

// ---- AI Setup ----

#[derive(Clone, Copy, Debug, ValueEnum)]
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
        /// Platform: telegram, discord, lark, feishu, openclaw
        #[arg(long)]
        platform: String,
        /// Bot token (hidden from help -- use --token-env instead)
        #[arg(long, hide = true)]
        bot_token: Option<String>,
        /// Read bot token from this environment variable
        #[arg(long)]
        token_env: Option<String>,
        /// Label for this bot
        #[arg(long)]
        label: String,
        /// Platform app ID (required for some platforms, e.g. Discord, Lark)
        #[arg(long)]
        app_id: Option<String>,
        /// Platform app secret (hidden from help -- use --app-secret-env instead)
        #[arg(long, hide = true)]
        app_secret: Option<String>,
        /// Read app secret from this environment variable
        #[arg(long)]
        app_secret_env: Option<String>,
        /// Platform public key (required for some platforms, e.g. Discord)
        #[arg(long)]
        public_key: Option<String>,
        /// Create this bot under the given org (you must be an admin of
        /// that org). Omit for a personal bot.
        #[arg(long, value_name = "ORG_ID")]
        org: Option<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// List registered bots
    List {
        /// List bots owned by the given org (admin-only). Omit for
        /// personal bots.
        #[arg(long, value_name = "ORG_ID")]
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
        #[arg(long, value_name = "ORG_ID")]
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
        #[arg(long, value_name = "ORG_ID")]
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
}
