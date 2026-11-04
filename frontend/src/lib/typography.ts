/** Shared semantic roles. text-N is always N/16rem; Display scales the root. */
export const TEXT_STYLES = {
  pageTitle: "text-28 font-bold",
  compactPageTitle: "text-22 font-bold",
  pageDescription: "text-14 font-normal",
  sectionHeading: "text-20 font-semibold",
  dialogTitle: "text-15 font-semibold",
  cardTitle: "text-15 font-semibold",
  rowTitle: "text-14 font-medium",
  groupHeading: "text-12 font-medium",
  body: "text-14 font-normal",
  secondary: "text-13 font-normal",
  meta: "text-13 font-normal",
  groupLabel: "text-12 font-medium",
  fieldLabel: "text-14 font-medium",
  validationError: "text-13 font-medium",
  navItem: "text-15 font-normal",
  activeNavItem: "text-15 font-medium",
  button: "text-14 font-medium",
  tab: "text-14 font-medium",
  input: "text-14 font-normal",
  menuItem: "text-14 font-normal",
  badge: "text-12 font-medium",
  tableHeader: "text-12 font-semibold",
  tableCell: "text-14 font-normal",
  tooltip: "text-13 font-normal",
  accountName: "text-14 font-medium",
  accountEmail: "text-12 font-normal",
  toastTitle: "text-15 font-medium",
  toastDescription: "text-14 font-normal",
  toastAction: "text-14 font-medium",
  chatMessage: "text-16 font-normal",
  composer: "text-16 font-normal",
  code: "text-14 font-normal",
  metric: "text-28 font-semibold",
} as const;

/** Canvas consumers (xterm) need pixels; read the actual token, never a fixed size. */
export function computedTextSize(token: number, root: HTMLElement = document.documentElement): number {
  const rem = Number.parseFloat(getComputedStyle(root).getPropertyValue(`--text-${token}`)) || token / 16;
  const rootSize = Number.parseFloat(getComputedStyle(root).fontSize) || 16;
  return rem * rootSize;
}
