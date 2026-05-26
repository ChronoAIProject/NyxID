/**
 * Central registry of tab values exposed via `?tab=` (and similar) URL
 * params. One source of truth for every page that surfaces tab state in the
 * URL — keeps validators, defaults, and deep-link hrefs aligned.
 */

export const SETTINGS_TABS = [
  "profile",
  "security",
  "sessions",
  "mcp",
  "privacy",
] as const;
export type SettingsTab = (typeof SETTINGS_TABS)[number];
export const SETTINGS_TAB_DEFAULT: SettingsTab = "profile";

export const CONSENTS_TABS = ["apps", "authorizations"] as const;
export type ConsentsTab = (typeof CONSENTS_TABS)[number];
export const CONSENTS_TAB_DEFAULT: ConsentsTab = "apps";

export const INTEGRATION_GUIDE_TABS = ["react", "core", "raw"] as const;
export type IntegrationGuideTab = (typeof INTEGRATION_GUIDE_TABS)[number];
export const INTEGRATION_GUIDE_TAB_DEFAULT: IntegrationGuideTab = "react";

export const ORG_DETAIL_TABS = [
  "members",
  "role-permissions",
  "invites",
  "approvals",
  "service-accounts",
  "developer-apps",
  "settings",
] as const;
export type OrgDetailTab = (typeof ORG_DETAIL_TABS)[number];
export const ORG_DETAIL_TAB_DEFAULT: OrgDetailTab = "members";

export const KEYS_TABS = ["services", "nyxid"] as const;
export type KeysTab = (typeof KEYS_TABS)[number];
export const KEYS_TAB_DEFAULT: KeysTab = "services";

export const KEYS_ACTIONS = ["add-service", "create-key"] as const;
export type KeysAction = (typeof KEYS_ACTIONS)[number];

export const INVITE_CODES_TABS = ["codes", "users"] as const;
export type InviteCodesTab = (typeof INVITE_CODES_TABS)[number];
export const INVITE_CODES_TAB_DEFAULT: InviteCodesTab = "codes";

export const AI_SETUP_SKILL_TABS = [
  "claude-code",
  "cursor",
  "codex",
  "openclaw",
  "chatgpt",
] as const;
export type AiSetupSkillTab = (typeof AI_SETUP_SKILL_TABS)[number];
export const AI_SETUP_SKILL_TAB_DEFAULT: AiSetupSkillTab = "claude-code";

/**
 * Validate an unknown search-param value against an allowlist. Returns the
 * value as the narrowed literal type when it matches, otherwise `fallback`.
 *
 * Typical use:
 *   const tab = parseTab(searchParams.tab, SETTINGS_TABS, SETTINGS_TAB_DEFAULT);
 */
export function parseTab<T extends string>(
  value: unknown,
  allowed: readonly T[],
  fallback: T,
): T {
  return typeof value === "string" && (allowed as readonly string[]).includes(value)
    ? (value as T)
    : fallback;
}

/**
 * Type guard variant of {@link parseTab} for cases where you want a boolean
 * check without a fallback.
 */
export function isValidTab<T extends string>(
  value: unknown,
  allowed: readonly T[],
): value is T {
  return typeof value === "string" && (allowed as readonly string[]).includes(value);
}
