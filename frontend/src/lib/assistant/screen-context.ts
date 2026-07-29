const NON_DASHBOARD_SURFACES = new Set([
  "assistant",
  "login",
  "register",
  "docs",
  "blog",
  "preview",
  "privacy",
  "terms",
  "oauth-consent",
  "error",
  "cli-auth",
  "cli",
  "ssh",
]);

const TWO_LEVEL_GROUPS = new Set(["admin", "approvals", "developer"]);

/** Collapse dashboard paths into a bounded screen-level association key. */
export function normalizeScreenKey(pathname: string): string | null {
  const segments = pathname.split("/").filter(Boolean);
  const first = segments[0];
  if (!first) return null;
  if (NON_DASHBOARD_SURFACES.has(first)) return null;

  if (first === "api-keys") return "/keys";
  if (first === "devices" && segments[1] === "onboard") {
    return "/devices/onboard";
  }
  if (first === "settings" && segments[1] === "consents") {
    return "/settings/consents";
  }
  if (first === "settings" && segments[1] === "authorizations") {
    return "/settings/consents";
  }
  if (TWO_LEVEL_GROUPS.has(first) && segments[1]) {
    return `/${first}/${segments[1]}`;
  }
  return `/${first}`;
}
