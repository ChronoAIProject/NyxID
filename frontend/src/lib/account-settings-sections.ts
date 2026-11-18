import { sectionNavigation } from "./section-navigation";
import { EDITABLE_COLORS, CONTRAST_CHECKS, type ColorKey } from "./theme-colors";

export const ACCOUNT_SETTINGS_SECTIONS = [
  "profile",
  "security",
  "security/delete",
  "sessions",
  "mcp",
  "display",
  "display/layout",
  "display/colors",
  "privacy",
] as const;
export type AccountSettingsSection = (typeof ACCOUNT_SETTINGS_SECTIONS)[number];
export const accountSettingsSections = sectionNavigation(ACCOUNT_SETTINGS_SECTIONS);
export const ACCOUNT_SETTINGS_TITLES: Record<AccountSettingsSection, string> = {
  profile: "Profile",
  security: "Security",
  "security/delete": "Delete account",
  sessions: "Sessions",
  mcp: "MCP",
  display: "Appearance",
  "display/layout": "Layout",
  "display/colors": "Colors",
  privacy: "Privacy",
};

/** Display has addressed peer views within one consistently named page. */
export function accountSettingsPageTitle(section: AccountSettingsSection) {
  return section === "display" || section.startsWith("display/")
    ? "Display"
    : ACCOUNT_SETTINGS_TITLES[section];
}
export const DISPLAY_COLOR_GROUPS: Record<string, readonly ColorKey[]> = {
  "Surfaces & chat bubbles": [
    "background",
    "sidebar",
    "card",
    "muted",
    "chat-bubble",
    "chat-bubble-foreground",
  ],
  "Text colors": ["foreground", "muted-foreground", "text-tertiary"],
  "Borders & accent": ["border", "input", "input-focus", "primary"],
};
export const ACCOUNT_FIELD_LABELS = {
  name: "Name",
  email: "Email",
  language: "Language",
  twofactorAuthentication: "Two-factor authentication",
  skipMfaForApprovedSignins: "Skip MFA for NyxID app sign-ins",
  currentPassword: "Current Password",
  newPassword: "New Password",
  confirmNewPassword: "Confirm New Password",
  deleteMyAccount: "Delete My Account",
  linkedAccounts: "Linked accounts",
  installToCursor: "Install to Cursor",
  installToClaudeCode: "Install to Claude Code",
  installToCodex: "Install to Codex",
  activeSessions: "Active sessions",
  revoke: "Revoke",
  allowAccountUsageAnalytics: "Allow account usage analytics",
  allowOnThisBrowser: "Allow on this browser",
  theme: "Theme",
  system: "System",
  light: "Light",
  dark: "Dark",
  motion: "Motion",
  followSystem: "Follow system",
  reduce: "Reduce",
  resetDisplaySettings: "Reset display settings",
  textSize: "Text size",
  spacing: "Spacing",
  chatContentWidth: "Chat content width",
  maximumComposerHeight: "Maximum composer height",
  mainSidebar: "Main sidebar",
  mainSidebarWidth: "Main sidebar width",
  assistantSidebarWidth: "Assistant sidebar width",
  expanded: "Expanded",
  collapsed: "Collapsed",
  expandOnHover: "Expand on hover",
} as const;
export const ACCOUNT_SETTINGS_LABELS: Partial<Record<AccountSettingsSection, readonly string[]>> = {
  profile: [ACCOUNT_FIELD_LABELS.name, ACCOUNT_FIELD_LABELS.email, ACCOUNT_FIELD_LABELS.language],
  security: [ACCOUNT_FIELD_LABELS.twofactorAuthentication, ACCOUNT_FIELD_LABELS.skipMfaForApprovedSignins, ACCOUNT_FIELD_LABELS.currentPassword, ACCOUNT_FIELD_LABELS.newPassword, ACCOUNT_FIELD_LABELS.confirmNewPassword, ACCOUNT_FIELD_LABELS.linkedAccounts],
  "security/delete": [ACCOUNT_FIELD_LABELS.deleteMyAccount],
  mcp: [ACCOUNT_FIELD_LABELS.installToCursor, ACCOUNT_FIELD_LABELS.installToClaudeCode, ACCOUNT_FIELD_LABELS.installToCodex],
  sessions: [ACCOUNT_FIELD_LABELS.activeSessions, ACCOUNT_FIELD_LABELS.revoke],
  privacy: [ACCOUNT_FIELD_LABELS.allowAccountUsageAnalytics, ACCOUNT_FIELD_LABELS.allowOnThisBrowser],
  display: [ACCOUNT_FIELD_LABELS.theme, ACCOUNT_FIELD_LABELS.system, ACCOUNT_FIELD_LABELS.light, ACCOUNT_FIELD_LABELS.dark, ACCOUNT_FIELD_LABELS.motion, ACCOUNT_FIELD_LABELS.followSystem, ACCOUNT_FIELD_LABELS.reduce, ACCOUNT_FIELD_LABELS.resetDisplaySettings, ACCOUNT_FIELD_LABELS.textSize, ACCOUNT_FIELD_LABELS.spacing],
  "display/layout": [ACCOUNT_FIELD_LABELS.chatContentWidth, ACCOUNT_FIELD_LABELS.maximumComposerHeight, ACCOUNT_FIELD_LABELS.mainSidebar, ACCOUNT_FIELD_LABELS.mainSidebarWidth, ACCOUNT_FIELD_LABELS.assistantSidebarWidth, ACCOUNT_FIELD_LABELS.expanded, ACCOUNT_FIELD_LABELS.collapsed, ACCOUNT_FIELD_LABELS.expandOnHover],
  "display/colors": [...EDITABLE_COLORS.map(color => color.label), ...CONTRAST_CHECKS.map(check => check.label)],
};
