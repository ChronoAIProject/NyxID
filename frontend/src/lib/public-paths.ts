/**
 * Routes that render before the browser session check completes.
 *
 * `/cli/pair` and `/login/device` must preserve their query strings until
 * their pages can route through login. Connect-link pages likewise preserve
 * their token or link ID and provider result through their own login redirect.
 * The exact OAuth popup routes must boot
 * immediately so opener severance and completion broadcasting do not depend
 * on an authenticated NyxID session. Do not broaden them to `/oauth/*`; those
 * paths belong to NyxID's backend IdP surface.
 */
export function isPublicPath(path: string): boolean {
  return (
    (import.meta.env.DEV && path === "/temp") ||
    path === "/" ||
    path === "/login" ||
    path === "/register" ||
    path === "/privacy" ||
    path === "/terms" ||
    path === "/blog" ||
    path.startsWith("/blog/") ||
    path === "/docs" ||
    path.startsWith("/docs/") ||
    path.startsWith("/preview/") ||
    path.startsWith("/error") ||
    path.startsWith("/oauth-consent") ||
    path === "/oauth-complete" ||
    path === "/oauth-launching" ||
    /^\/connect\/(?:return\/)?[^/]+\/?$/.test(path) ||
    path === "/cli-auth" ||
    path === "/cli/pair" ||
    path === "/login/device" ||
    path === "/login/agent-key"
  );
}
