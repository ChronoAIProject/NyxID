export function accountWebUrl(frontend: string, path: string): string {
  const base = new URL(frontend);
  if (
    !["https:", "http:"].includes(base.protocol) ||
    base.username ||
    base.password
  )
    throw new Error(
      "NyxID website is not configured. Contact your administrator.",
    );
  const url = new URL(path, base.origin);
  if (url.origin !== base.origin)
    throw new Error("Invalid NyxID website destination.");
  return url.href;
}
export const ORGANIZATION_WEB_ACTIONS = [
  { label: "Members & roles", tab: "members", adminOnly: false },
  { label: "Invitations", tab: "invites", adminOnly: true },
  { label: "Organization approvals", tab: "approvals", adminOnly: true },
  { label: "Role permissions", tab: "role-permissions", adminOnly: true },
  {
    label: "Organization settings & deletion",
    tab: "settings",
    adminOnly: true,
  },
] as const;
