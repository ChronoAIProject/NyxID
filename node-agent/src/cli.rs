use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(name = "nyxid-node", about = "NyxID credential node agent")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Log level (trace, debug, info, warn, error)
    #[arg(long, global = true)]
    pub log_level: Option<String>,
}

#[derive(Subcommand)]
pub enum Commands {
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
    },

    /// Start the node agent (connect and serve)
    Start {
        /// Path to config directory
        #[arg(long)]
        config: Option<String>,
    },

    /// Show node connection status
    Status {
        /// Path to config directory
        #[arg(long)]
        config: Option<String>,
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
    },

    /// Manage local credentials
    Credentials {
        #[command(subcommand)]
        command: CredentialCommands,

        /// Path to config directory
        #[arg(long)]
        config: Option<String>,
    },

    /// Migrate secret storage from file to OS keychain (or vice versa)
    Migrate {
        /// Target backend: "keychain" or "file"
        #[arg(long)]
        to: String,

        /// Path to config directory
        #[arg(long)]
        config: Option<String>,
    },

    /// Show version information
    Version,
}

#[derive(Subcommand)]
pub enum CredentialCommands {
    /// Add a credential for a service (prompts for the secret value securely)
    Add {
        /// Service slug (e.g., "openai", "github-api")
        #[arg(long)]
        service: String,

        /// Header name to inject (e.g., "Authorization"). The value will be prompted securely.
        #[arg(long)]
        header: Option<String>,

        /// Query parameter name to inject (e.g., "api_key"). The value will be prompted securely.
        #[arg(long)]
        query_param: Option<String>,

        /// How to format the prompted secret before storing it.
        #[arg(long, value_enum, default_value_t = CredentialSecretFormat::Raw)]
        secret_format: CredentialSecretFormat,

        /// Inline secret value (skips interactive prompt; NOT recommended -- visible in shell history)
        #[arg(long, hide = true)]
        value: Option<String>,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum CredentialSecretFormat {
    /// Store the secret exactly as entered.
    Raw,
    /// Prefix the secret with "Bearer ".
    Bearer,
    /// Base64-encode "username:password" and prefix it with "Basic ".
    Basic,
}
