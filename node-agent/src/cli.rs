use clap::{Parser, Subcommand};

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
    /// Add a credential for a service
    Add {
        /// Service slug (e.g., "openai", "github-api")
        #[arg(long)]
        service: String,

        /// Header to inject (e.g., "Authorization: Bearer sk-...")
        #[arg(long)]
        header: Option<String>,

        /// Query parameter to inject (e.g., "api_key=sk-...")
        #[arg(long)]
        query_param: Option<String>,
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
