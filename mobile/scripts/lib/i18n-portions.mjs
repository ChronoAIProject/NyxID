/**
 * Ordered path -> area map for the mobile catalogs. The first matching pattern
 * wins. Each feature directory owns its own area, the navigation shell owns
 * `app`, and remaining shared code (components, hooks, lib, theme, i18n) maps to
 * `shared`, which also holds keys used by several areas. Keep this list stable:
 * the adoption tool uses it to place keys added on pre-split branches.
 */
const FEATURES = [
  "account", "activity", "agents", "assistant", "auth", "automations", "billing", "display",
  "groups", "legal", "machines", "notifications", "onboarding", "plugins", "services", "ssh",
];

export const PORTIONS = [
  ...FEATURES.map((name) => ({ name, patterns: [new RegExp(`^src/features/${name}/`)] })),
  { name: "app", patterns: [/^src\/app\//] },
  { name: "shared", patterns: [/^src\//] },
  { name: "unattributed", patterns: [] },
];

export function portionForPath(file) {
  return PORTIONS.find((portion) => portion.patterns.some((pattern) => pattern.test(file)))?.name ?? "shared";
}
