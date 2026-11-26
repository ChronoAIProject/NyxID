import { localizedError } from "../../i18n/localizedError";
import { translateApp } from "../../i18n/appTranslation";
export function accountWebUrl(frontend: string, path: string): string {
  const base = new URL(frontend);
  if (
    !["https:", "http:"].includes(base.protocol) ||
    base.username ||
    base.password
  )
    throw localizedError("NyxID website is not configured. Contact your administrator.");
  const url = new URL(path, base.origin);
  if (url.origin !== base.origin)
    throw localizedError("Invalid NyxID website destination.");
  return url.href;
}
export const ORGANIZATION_WEB_ACTIONS = [
  {
    get label() {
      return translateApp("Members & roles");
    }, tab: "members", adminOnly: false },
  {
    get label() {
      return translateApp("Invitations");
    }, tab: "invites", adminOnly: true },
  {
    get label() {
      return translateApp("Organization approvals");
    }, tab: "approvals", adminOnly: true },
  {
    get label() {
      return translateApp("Role permissions");
    }, tab: "role-permissions", adminOnly: true },
  {
    get label() {
      return translateApp("Organization settings & deletion");
    },
    tab: "settings",
    adminOnly: true,
  },
] as const;
