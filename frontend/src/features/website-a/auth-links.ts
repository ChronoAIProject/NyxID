import publicRoutes from "./generated/routes.json";
import { solutionStartTarget } from "./solution-availability";
// Preserve the landing page's existing invitation attribution and full query.
const REDDIT_DEFAULT_INVITE_CODE = "NYX-QJGT6NHN";

export function injectRedditDefaultCode() {
  const params = new URLSearchParams(window.location.search);
  if (params.get("code")) return;
  let referrerHost = "";
  try {
    referrerHost = new URL(document.referrer).hostname.toLowerCase();
  } catch {
    // A direct visit has no referrer URL.
  }
  if (
    /(^|\.)reddit\.com$|(^|\.)redd\.it$/.test(referrerHost) ||
    params.get("utm_source")?.toLowerCase() === "reddit"
  ) {
    params.set("code", REDDIT_DEFAULT_INVITE_CODE);
    window.history.replaceState(
      window.history.state,
      "",
      `${window.location.pathname}?${params.toString()}${window.location.hash}`,
    );
  }
}

export function authHref(
  kind: "login" | "register",
  search = window.location.search,
) {
  return `/${kind}${search}`;
}

export function siteHref(pathname: string, search = window.location.search) {
  const destination = new URL(pathname, window.location.origin);
  const params = new URLSearchParams(search);
  destination.searchParams.forEach((value, key) => params.set(key, value));
  const query = params.toString();
  return `${destination.pathname}${query ? `?${query}` : ""}${destination.hash}`;
}

/** Only recognized public prototype paths may leave the fragment namespace. */
export function legacyDestination(
  hash: string,
  search = window.location.search,
): string | null {
  if (!hash.startsWith("#/")) return null;
  const fragment = hash.slice(1);
  const queryStart = fragment.indexOf("?");
  const pathname = queryStart === -1 ? fragment : fragment.slice(0, queryStart);
  const query = queryStart === -1 ? "" : fragment.slice(queryStart + 1);
  if (pathname === "/login" || pathname === "/signup") {
    const target = pathname === "/login" ? "/login" : "/register";
    // Real auth links retain both contexts; destination parameters take precedence.
    return siteHref(target + (query ? `?${query}` : ""), search);
  }
  if (pathname === "/start" || pathname === "/done") {
    const solution = solutionStartTarget(query);
    if (solution) return siteHref(solution, search);
    // Mock provisioning fragments (including `t`) are not real auth state.
    // Preserve only the original outer query when entering real registration.
    return authHref("register", search);
  }
  if (!publicRoutes.includes(pathname)) return null;
  return siteHref(pathname + (query ? `?${query}` : ""), search);
}
