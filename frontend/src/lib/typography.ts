/** Shared semantic roles. text-N is always N/16rem; Display scales the root. */
export const TEXT_STYLES = {
  pageTitle: "type-page-title",
  compactPageTitle: "type-compact-page-title",
  pageDescription: "type-page-description",
  sectionHeading: "type-section-heading",
  dialogTitle: "type-dialog-title",
  cardTitle: "type-card-title",
  rowTitle: "type-row-title",
  groupHeading: "type-group-heading",
  body: "type-body",
  secondary: "type-secondary",
  meta: "type-meta",
  groupLabel: "type-group-label",
  countBadge: "type-count-badge",
  fieldLabel: "type-field-label",
  avatarInitials: "type-avatar-initials",
  avatarInitialsLarge: "type-avatar-initials-large",
  avatarInitialsHeader: "type-avatar-initials-header",
  validationError: "type-validation-error",
  navItem: "type-nav-item",
  activeNavItem: "type-active-nav-item",
  button: "type-button",
  tab: "type-tab",
  input: "type-input",
  menuItem: "type-menu-item",
  badge: "type-badge",
  tableHeader: "type-table-header",
  tableCell: "type-table-cell",
  tooltip: "type-tooltip",
  accountName: "type-account-name",
  accountEmail: "type-account-email",
  toastTitle: "type-toast-title",
  toastDescription: "type-toast-description",
  toastAction: "type-toast-action",
  chatMessage: "type-chat-message",
  chatMention: "type-chat-mention",
  composer: "type-composer",
  code: "type-code",
  metric: "type-metric",
  dataTotal: "type-data-total",
  brandMonogram: "type-brand-monogram",
  brandLetter: "type-brand-letter",
  secondaryEmphasis: "type-secondary-emphasis",
  secondaryStrong: "type-secondary-strong",
  bodyStrong: "type-body-strong",
  subheading: "type-subheading",
  prose: "type-prose",
  proseHeading: "type-prose-heading",
  proseSubheading: "type-prose-subheading",
  proseMinorHeading: "type-prose-minor-heading",
} as const;

/** Canvas consumers (xterm) need pixels; read the actual token, never a fixed size. */
export function computedTextSize(token: number, root: HTMLElement = document.documentElement): number {
  const rem = Number.parseFloat(getComputedStyle(root).getPropertyValue(`--text-${token}`)) || token / 16;
  const rootSize = Number.parseFloat(getComputedStyle(root).fontSize) || 16;
  return rem * rootSize;
}

/** Library renderers use the same role line box as DOM text. */
export function computedTextLineHeight(role: "body" | "prose" = "body"): number {
  return Number.parseFloat(getComputedStyle(document.documentElement).getPropertyValue(`--line-${role}`)) || (role === "prose" ? 1.625 : 1.5);
}
