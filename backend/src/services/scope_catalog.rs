//! Curated, per-provider OAuth scope catalogs (NyxID#917 follow-up).
//!
//! The OAuth request path is generic — NyxID forwards whatever scopes the
//! caller asks for and lets the upstream provider accept or reject them. But
//! a user can only type a scope they already know. This module bakes a
//! curated "menu" of each provider's notable scopes into the system so the
//! connect UIs (dashboard add-key dialog and CLI pair wizard) can render them
//! as selectable pills with human labels, alongside a free-form "add more"
//! field for anything not listed here.
//!
//! Design notes:
//! - **Static, not stored.** The catalog lives in code and is projected onto
//!   the catalog API (`GET /catalog/{slug}`) keyed by *provider* slug. This
//!   sidesteps the insert-only provider-seed migration problem and matches the
//!   "baked into the system" intent. Admin-editable-in-DB is a later layer.
//! - **Not authoritative or exhaustive.** Platforms add scopes faster than we
//!   reseed; the provider still arbitrates at consent time. The custom field
//!   is the escape hatch. Coverage is deep for major platforms, lighter for
//!   niche ones.
//! - **Defaults are separate.** A provider's `default_scopes` already ship via
//!   the catalog response; the UI unions them with this list and pre-selects
//!   the defaults. Entries here may overlap defaults — that's fine, the UI
//!   dedupes.
//! - `sensitive: true` flags write/admin/DM-grade scopes so the UI can mark
//!   them visually. It carries no server-side enforcement.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ScopeCatalogEntry {
    /// The exact scope string sent to the provider (e.g. `media.write`).
    pub scope: String,
    /// Short human label for the pill (e.g. "Upload media").
    pub label: String,
    /// One-line explanation of what granting it allows.
    pub description: String,
    /// Write/admin/DM-grade scope — UI may emphasize it. No server effect.
    #[serde(default)]
    pub sensitive: bool,
}

/// How safely a granted scope can be *removed* from an existing connection.
/// Drives the Permissions panel: whether deselecting a granted scope is
/// offered, and whether NyxID can clean up the old grant at the provider or
/// the user must do it manually. (Adding scopes works on every provider, so
/// there's no "add" capability.)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ScopeRemoval {
    /// Re-authorizing with a narrower set yields a narrower token AND the
    /// provider exposes a token-revocation endpoint, so NyxID cancels the old
    /// grant automatically. Fully hands-off.
    Auto,
    /// Re-auth narrows the token NyxID holds, but there's no programmatic
    /// revoke — the prior grant lingers until the user removes it in the
    /// provider's own settings. UI allows removal with a "revoke at <provider>"
    /// note.
    Manual,
    /// Removal can't be achieved by re-auth because the provider tracks scopes
    /// at the grant level as a union (e.g. GitHub). Needs a provider-specific
    /// teardown not yet implemented; UI keeps granted scopes locked.
    Unsupported,
}

/// Per-provider scope-removal capability. `auto` = providers with a seeded
/// `revocation_url` and standard downscope-on-reauth; `manual` = downscope
/// works but no programmatic revoke; `unsupported` = grant-level union
/// (GitHub) where re-auth can't narrow. Unknown/uncurated providers default to
/// `manual` (allow removal, advise verifying at the provider) — the long tail
/// still needs per-provider verification before promising `auto`.
pub fn removal_capability(slug: &str) -> ScopeRemoval {
    match slug {
        "twitter" | "google" | "google-cloud" | "discord" | "slack" | "tiktok" | "twitch"
        | "reddit" => ScopeRemoval::Auto,
        "facebook" | "spotify" | "linkedin" | "microsoft" | "lark" | "feishu" => {
            ScopeRemoval::Manual
        }
        "github" => ScopeRemoval::Unsupported,
        _ => ScopeRemoval::Manual,
    }
}

/// Curated available-scope menu for a provider, keyed by its `ProviderConfig`
/// slug (e.g. `twitter`, `google`, `github`). Returns `None` for providers
/// with no curated catalog (api_key providers, `openai`-format device-code
/// providers that reject a `scope` param, and anything not yet curated) — the
/// UI then falls back to free-form entry only.
pub fn for_provider(slug: &str) -> Option<Vec<ScopeCatalogEntry>> {
    let entries: &[(&str, &str, &str, bool)] = match slug {
        "twitter" => TWITTER,
        "google" => GOOGLE,
        "google-cloud" => GOOGLE_CLOUD,
        "github" => GITHUB,
        "facebook" => FACEBOOK,
        "discord" => DISCORD,
        "spotify" => SPOTIFY,
        "linkedin" => LINKEDIN,
        "slack" => SLACK,
        "microsoft" => MICROSOFT,
        "tiktok" => TIKTOK,
        "twitch" => TWITCH,
        "reddit" => REDDIT,
        "lark" | "feishu" => LARK,
        _ => return None,
    };
    Some(
        entries
            .iter()
            .map(|(scope, label, description, sensitive)| ScopeCatalogEntry {
                scope: (*scope).to_string(),
                label: (*label).to_string(),
                description: (*description).to_string(),
                sensitive: *sensitive,
            })
            .collect(),
    )
}

// Tuple shape: (scope, label, description, sensitive)

const TWITTER: &[(&str, &str, &str, bool)] = &[
    (
        "tweet.read",
        "Read posts",
        "Read posts and timelines.",
        false,
    ),
    (
        "tweet.write",
        "Write posts",
        "Create and delete posts on your behalf.",
        true,
    ),
    (
        "tweet.moderate.write",
        "Moderate replies",
        "Hide and unhide replies to your posts.",
        true,
    ),
    (
        "users.read",
        "Read profile",
        "Read profile info for you and other accounts.",
        false,
    ),
    (
        "follows.read",
        "Read follows",
        "See who you follow and who follows you.",
        false,
    ),
    (
        "follows.write",
        "Manage follows",
        "Follow and unfollow accounts on your behalf.",
        true,
    ),
    (
        "offline.access",
        "Stay connected",
        "Refresh access without re-authorizing (long-lived token).",
        false,
    ),
    ("like.read", "Read likes", "See posts you've liked.", false),
    (
        "like.write",
        "Manage likes",
        "Like and unlike posts on your behalf.",
        true,
    ),
    (
        "list.read",
        "Read lists",
        "Read your lists and their members.",
        false,
    ),
    (
        "list.write",
        "Manage lists",
        "Create and edit lists on your behalf.",
        true,
    ),
    (
        "bookmark.read",
        "Read bookmarks",
        "See your bookmarked posts.",
        false,
    ),
    (
        "bookmark.write",
        "Manage bookmarks",
        "Add and remove bookmarks on your behalf.",
        true,
    ),
    (
        "block.read",
        "Read blocks",
        "See accounts you've blocked.",
        false,
    ),
    (
        "block.write",
        "Manage blocks",
        "Block and unblock accounts on your behalf.",
        true,
    ),
    (
        "mute.read",
        "Read mutes",
        "See accounts you've muted.",
        false,
    ),
    (
        "mute.write",
        "Manage mutes",
        "Mute and unmute accounts on your behalf.",
        true,
    ),
    (
        "space.read",
        "Read Spaces",
        "Read details about Spaces.",
        false,
    ),
    ("dm.read", "Read DMs", "Read your direct messages.", true),
    (
        "dm.write",
        "Send DMs",
        "Send direct messages on your behalf.",
        true,
    ),
    (
        "media.write",
        "Upload media",
        "Upload images and video (required for POST /2/media/upload).",
        true,
    ),
];

// Google scopes are full URLs. Curated common set across identity + Workspace.
const GOOGLE: &[(&str, &str, &str, bool)] = &[
    (
        "openid",
        "Sign-in (OpenID)",
        "Authenticate your Google identity.",
        false,
    ),
    (
        "email",
        "Email address",
        "See your primary Google email address.",
        false,
    ),
    (
        "profile",
        "Basic profile",
        "See your name and profile picture.",
        false,
    ),
    (
        "https://www.googleapis.com/auth/drive.readonly",
        "Drive (read)",
        "Read files in your Google Drive.",
        false,
    ),
    (
        "https://www.googleapis.com/auth/drive.file",
        "Drive (app files)",
        "Manage only files this app creates in Drive.",
        false,
    ),
    (
        "https://www.googleapis.com/auth/drive",
        "Drive (full)",
        "Full read/write access to all Drive files.",
        true,
    ),
    (
        "https://www.googleapis.com/auth/spreadsheets.readonly",
        "Sheets (read)",
        "Read your Google Sheets.",
        false,
    ),
    (
        "https://www.googleapis.com/auth/spreadsheets",
        "Sheets (read/write)",
        "Read and edit your Google Sheets.",
        true,
    ),
    (
        "https://www.googleapis.com/auth/documents.readonly",
        "Docs (read)",
        "Read your Google Docs.",
        false,
    ),
    (
        "https://www.googleapis.com/auth/documents",
        "Docs (read/write)",
        "Read and edit your Google Docs.",
        true,
    ),
    (
        "https://www.googleapis.com/auth/gmail.readonly",
        "Gmail (read)",
        "Read your email messages and settings.",
        true,
    ),
    (
        "https://www.googleapis.com/auth/gmail.send",
        "Gmail (send)",
        "Send email on your behalf.",
        true,
    ),
    (
        "https://www.googleapis.com/auth/gmail.modify",
        "Gmail (modify)",
        "Read, compose, and modify email (no permanent delete).",
        true,
    ),
    (
        "https://www.googleapis.com/auth/calendar.readonly",
        "Calendar (read)",
        "Read your calendars and events.",
        false,
    ),
    (
        "https://www.googleapis.com/auth/calendar",
        "Calendar (read/write)",
        "Manage your calendars and events.",
        true,
    ),
];

const GOOGLE_CLOUD: &[(&str, &str, &str, bool)] = &[
    (
        "https://www.googleapis.com/auth/cloud-platform.read-only",
        "Cloud (read)",
        "Read-only access to Google Cloud resources.",
        false,
    ),
    (
        "https://www.googleapis.com/auth/cloud-platform",
        "Cloud (full)",
        "Full management of Google Cloud resources.",
        true,
    ),
];

const GITHUB: &[(&str, &str, &str, bool)] = &[
    (
        "read:user",
        "Read profile",
        "Read your profile data.",
        false,
    ),
    (
        "user:email",
        "Email addresses",
        "Read your email addresses.",
        false,
    ),
    (
        "repo",
        "Repositories (full)",
        "Full control of private and public repositories.",
        true,
    ),
    (
        "public_repo",
        "Public repos",
        "Access public repositories only.",
        false,
    ),
    (
        "repo:status",
        "Commit statuses",
        "Read and write commit statuses.",
        false,
    ),
    (
        "read:org",
        "Read org",
        "Read org membership and teams.",
        false,
    ),
    (
        "write:org",
        "Manage org",
        "Manage org membership and teams.",
        true,
    ),
    ("gist", "Gists", "Create and edit gists.", true),
    (
        "workflow",
        "Actions workflows",
        "Update GitHub Actions workflow files.",
        true,
    ),
    (
        "notifications",
        "Notifications",
        "Read and manage your notifications.",
        false,
    ),
    (
        "read:packages",
        "Packages (read)",
        "Download packages from GitHub Packages.",
        false,
    ),
    (
        "write:packages",
        "Packages (write)",
        "Upload packages to GitHub Packages.",
        true,
    ),
    (
        "delete_repo",
        "Delete repos",
        "Delete repositories you administer.",
        true,
    ),
];

const FACEBOOK: &[(&str, &str, &str, bool)] = &[
    (
        "public_profile",
        "Basic profile",
        "See your public profile.",
        false,
    ),
    ("email", "Email address", "See your email address.", false),
    (
        "pages_show_list",
        "List Pages",
        "See the list of Pages you manage.",
        false,
    ),
    (
        "pages_read_engagement",
        "Page engagement",
        "Read content and engagement on your Pages.",
        false,
    ),
    (
        "pages_manage_posts",
        "Manage Page posts",
        "Create, edit, and delete posts on your Pages.",
        true,
    ),
    (
        "pages_messaging",
        "Page messaging",
        "Send and receive messages on your Pages.",
        true,
    ),
    (
        "business_management",
        "Business assets",
        "Manage Business Manager assets.",
        true,
    ),
];

const DISCORD: &[(&str, &str, &str, bool)] = &[
    (
        "identify",
        "Identity",
        "See your username, avatar, and ID.",
        false,
    ),
    ("email", "Email address", "See your email address.", false),
    (
        "guilds",
        "Servers list",
        "See the servers you're in.",
        false,
    ),
    (
        "guilds.members.read",
        "Server membership",
        "Read your member info in servers.",
        false,
    ),
    (
        "guilds.join",
        "Join servers",
        "Add you to a server on your behalf.",
        true,
    ),
    (
        "connections",
        "Linked accounts",
        "See your linked third-party accounts.",
        false,
    ),
    (
        "applications.commands",
        "Slash commands",
        "Use application commands in servers.",
        false,
    ),
    (
        "bot",
        "Bot",
        "Add a bot to a server (with the bot permissions you grant).",
        true,
    ),
    (
        "messages.read",
        "Read messages",
        "Read messages in channels you can access.",
        true,
    ),
];

const SPOTIFY: &[(&str, &str, &str, bool)] = &[
    (
        "user-read-email",
        "Email address",
        "Read your email address.",
        false,
    ),
    (
        "user-read-private",
        "Account details",
        "Read your subscription and country.",
        false,
    ),
    (
        "playlist-read-private",
        "Private playlists",
        "Read your private playlists.",
        false,
    ),
    (
        "playlist-modify-public",
        "Edit public playlists",
        "Create and edit your public playlists.",
        true,
    ),
    (
        "playlist-modify-private",
        "Edit private playlists",
        "Create and edit your private playlists.",
        true,
    ),
    (
        "user-library-read",
        "Library (read)",
        "Read your saved tracks and albums.",
        false,
    ),
    (
        "user-library-modify",
        "Library (modify)",
        "Save and remove tracks and albums.",
        true,
    ),
    (
        "user-top-read",
        "Top artists/tracks",
        "Read your top artists and tracks.",
        false,
    ),
    (
        "user-read-playback-state",
        "Playback state",
        "Read your current playback state.",
        false,
    ),
    (
        "user-modify-playback-state",
        "Control playback",
        "Control playback on your devices.",
        true,
    ),
    (
        "streaming",
        "Stream audio",
        "Play content via the Web Playback SDK.",
        true,
    ),
];

const LINKEDIN: &[(&str, &str, &str, bool)] = &[
    (
        "openid",
        "Sign-in (OpenID)",
        "Authenticate your LinkedIn identity.",
        false,
    ),
    (
        "profile",
        "Basic profile",
        "Read your name and profile.",
        false,
    ),
    (
        "email",
        "Email address",
        "Read your primary email address.",
        false,
    ),
    (
        "w_member_social",
        "Post as you",
        "Create posts, comments, and reactions on your behalf.",
        true,
    ),
];

const SLACK: &[(&str, &str, &str, bool)] = &[
    (
        "users:read",
        "Read users",
        "View people in the workspace.",
        false,
    ),
    (
        "users:read.email",
        "User emails",
        "View email addresses of workspace members.",
        false,
    ),
    (
        "channels:read",
        "Read channels",
        "View basic info about public channels.",
        false,
    ),
    (
        "channels:history",
        "Channel history",
        "View messages in public channels.",
        true,
    ),
    (
        "chat:write",
        "Send messages",
        "Send messages on your behalf.",
        true,
    ),
    (
        "files:read",
        "Read files",
        "View files shared in the workspace.",
        false,
    ),
    (
        "files:write",
        "Upload files",
        "Upload and edit files on your behalf.",
        true,
    ),
    (
        "groups:read",
        "Read private channels",
        "View basic info about private channels.",
        true,
    ),
    (
        "im:history",
        "DM history",
        "View your direct message history.",
        true,
    ),
    (
        "reactions:write",
        "Add reactions",
        "Add and remove emoji reactions.",
        false,
    ),
];

const MICROSOFT: &[(&str, &str, &str, bool)] = &[
    (
        "openid",
        "Sign-in (OpenID)",
        "Authenticate your Microsoft identity.",
        false,
    ),
    ("email", "Email address", "See your email address.", false),
    (
        "profile",
        "Basic profile",
        "See your name and profile.",
        false,
    ),
    (
        "offline_access",
        "Stay connected",
        "Refresh access without re-authorizing.",
        false,
    ),
    (
        "User.Read",
        "Read your profile",
        "Read your Microsoft profile.",
        false,
    ),
    ("Mail.Read", "Mail (read)", "Read your mail.", true),
    (
        "Mail.Send",
        "Mail (send)",
        "Send mail on your behalf.",
        true,
    ),
    (
        "Files.Read",
        "Files (read)",
        "Read your OneDrive files.",
        false,
    ),
    (
        "Files.ReadWrite",
        "Files (read/write)",
        "Read and write your OneDrive files.",
        true,
    ),
    (
        "Calendars.Read",
        "Calendar (read)",
        "Read your calendars.",
        false,
    ),
    (
        "Calendars.ReadWrite",
        "Calendar (read/write)",
        "Manage your calendars.",
        true,
    ),
];

const TIKTOK: &[(&str, &str, &str, bool)] = &[
    (
        "user.info.basic",
        "Basic profile",
        "Read your display name and avatar.",
        false,
    ),
    (
        "user.info.profile",
        "Profile details",
        "Read your profile bio and link.",
        false,
    ),
    (
        "user.info.stats",
        "Profile stats",
        "Read your follower and like counts.",
        false,
    ),
    (
        "video.list",
        "List videos",
        "Read your public videos.",
        false,
    ),
    (
        "video.upload",
        "Upload videos",
        "Upload videos to your account (draft).",
        true,
    ),
    (
        "video.publish",
        "Publish videos",
        "Publish videos directly to your account.",
        true,
    ),
];

const TWITCH: &[(&str, &str, &str, bool)] = &[
    (
        "user:read:email",
        "Email address",
        "Read your email address.",
        false,
    ),
    (
        "user:read:follows",
        "Read follows",
        "Read the channels you follow.",
        false,
    ),
    (
        "channel:read:subscriptions",
        "Read subscriptions",
        "Read your channel's subscribers.",
        true,
    ),
    ("chat:read", "Read chat", "Read chat messages.", false),
    (
        "chat:edit",
        "Send chat",
        "Send chat messages on your behalf.",
        true,
    ),
    (
        "clips:edit",
        "Manage clips",
        "Create clips on your behalf.",
        true,
    ),
    (
        "channel:manage:broadcast",
        "Manage broadcast",
        "Update your channel's title and category.",
        true,
    ),
];

const REDDIT: &[(&str, &str, &str, bool)] = &[
    ("identity", "Identity", "Read your account info.", false),
    ("read", "Read content", "Read posts and comments.", false),
    (
        "mysubreddits",
        "Your subreddits",
        "Read the subreddits you're subscribed to.",
        false,
    ),
    (
        "submit",
        "Submit posts",
        "Submit posts and comments on your behalf.",
        true,
    ),
    (
        "edit",
        "Edit content",
        "Edit your posts and comments.",
        true,
    ),
    ("vote", "Vote", "Cast votes on your behalf.", true),
    (
        "save",
        "Save content",
        "Save and unsave posts and comments.",
        false,
    ),
    (
        "history",
        "Read history",
        "Read your voting and posting history.",
        false,
    ),
    (
        "subscribe",
        "Manage subscriptions",
        "Subscribe to and unsubscribe from subreddits.",
        true,
    ),
];

// Lark / Feishu share the same scope grammar.
const LARK: &[(&str, &str, &str, bool)] = &[
    (
        "contact:user.base:readonly",
        "Basic profile",
        "Read basic user profile (name, avatar).",
        false,
    ),
    (
        "contact:user.email:readonly",
        "Email address",
        "Read the user's email address.",
        false,
    ),
    (
        "contact:user.employee_id:readonly",
        "Employee ID",
        "Read the user's employee ID.",
        false,
    ),
    (
        "offline_access",
        "Stay connected",
        "Refresh access without re-authorizing.",
        false,
    ),
    ("im:message", "Messages", "Read and send messages.", true),
    (
        "im:message:send_as_bot",
        "Send as bot",
        "Send messages as the bot.",
        true,
    ),
    ("docs:doc:readonly", "Docs (read)", "Read documents.", false),
    (
        "docx:document",
        "Docs (read/write)",
        "Read and edit documents.",
        true,
    ),
    (
        "drive:drive:readonly",
        "Drive (read)",
        "Read files in Drive.",
        false,
    ),
    (
        "drive:drive",
        "Drive (read/write)",
        "Read and write files in Drive.",
        true,
    ),
    (
        "sheets:spreadsheet:readonly",
        "Sheets (read)",
        "Read spreadsheets.",
        false,
    ),
    (
        "sheets:spreadsheet",
        "Sheets (read/write)",
        "Read and edit spreadsheets.",
        true,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removal_capability_matches_provider_revoke_support() {
        // revoke-endpoint providers → auto
        for slug in [
            "twitter",
            "google",
            "google-cloud",
            "discord",
            "slack",
            "tiktok",
            "twitch",
            "reddit",
        ] {
            assert_eq!(removal_capability(slug), ScopeRemoval::Auto, "{slug}");
        }
        // downscope works but no programmatic revoke → manual
        for slug in [
            "facebook",
            "spotify",
            "linkedin",
            "microsoft",
            "lark",
            "feishu",
        ] {
            assert_eq!(removal_capability(slug), ScopeRemoval::Manual, "{slug}");
        }
        // grant-level union, re-auth can't narrow → unsupported
        assert_eq!(removal_capability("github"), ScopeRemoval::Unsupported);
        // unknown providers default to manual (allow, advise verifying)
        assert_eq!(
            removal_capability("some-future-provider"),
            ScopeRemoval::Manual
        );
    }

    #[test]
    fn known_oauth_providers_have_catalogs() {
        for slug in [
            "twitter",
            "google",
            "google-cloud",
            "github",
            "facebook",
            "discord",
            "spotify",
            "linkedin",
            "slack",
            "microsoft",
            "tiktok",
            "twitch",
            "reddit",
            "lark",
            "feishu",
        ] {
            let cat = for_provider(slug).unwrap_or_else(|| panic!("missing catalog for {slug}"));
            assert!(!cat.is_empty(), "empty catalog for {slug}");
        }
    }

    #[test]
    fn lark_and_feishu_share_a_catalog() {
        assert_eq!(for_provider("lark"), for_provider("feishu"));
    }

    #[test]
    fn unknown_and_scopeless_providers_return_none() {
        // api_key providers and openai-format device-code (Codex) have no menu.
        assert!(for_provider("openai").is_none());
        assert!(for_provider("openai-codex").is_none());
        assert!(for_provider("anthropic").is_none());
        assert!(for_provider("does-not-exist").is_none());
    }

    #[test]
    fn twitter_includes_media_write_as_sensitive() {
        let cat = for_provider("twitter").unwrap();
        let media = cat.iter().find(|e| e.scope == "media.write").unwrap();
        assert!(media.sensitive);
    }

    #[test]
    fn entries_have_nonempty_labels_and_descriptions() {
        for slug in ["twitter", "google", "github", "slack"] {
            for e in for_provider(slug).unwrap() {
                assert!(!e.scope.is_empty());
                assert!(!e.label.is_empty(), "empty label for {} in {slug}", e.scope);
                assert!(
                    !e.description.is_empty(),
                    "empty desc for {} in {slug}",
                    e.scope
                );
            }
        }
    }
}
