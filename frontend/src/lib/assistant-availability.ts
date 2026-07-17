import type { User } from "@/types/api";
import { api } from "@/lib/api-client";
import { FEATURE_FLAG } from "@/lib/feature-flags";

type AssistantAvailabilityUser = Pick<User, "capabilities"> | null;

interface AuthSnapshot {
  readonly isAuthenticated: boolean;
  readonly isLoading: boolean;
  readonly user: User | null;
}

export type AssistantEntryDecision =
  | "allow"
  | "redirect-login"
  | "redirect-dashboard";

/**
 * Entry decision for the /assistant* route guards. The server-verified user
 * from `fetchAssistantAccessUser` wins when present; when that re-fetch
 * failed transiently (non-401), fall back to the already-authenticated
 * store snapshot so a network hiccup never bounces an entitled user off the
 * page — the flag only gates UI, the backend authorizes every API call.
 * With no user from either source this stays fail-closed (flag-off).
 */
export function hasAssistantAccess(
  fetchedUser: AssistantAvailabilityUser,
  snapshotUser: AssistantAvailabilityUser,
): boolean {
  const user = fetchedUser ?? snapshotUser;
  return (user?.capabilities?.enabled_features ?? []).includes(
    FEATURE_FLAG.AI_ASSISTANT,
  );
}

/**
 * Server-verified flag lookup for the /assistant* route guards: re-fetches
 * `/users/me` so route entry reflects the backend's current flag resolution
 * instead of the boot-time auth-store snapshot (a flag disabled mid-session
 * must lock the route on the next navigation, not the next hard reload).
 *
 * Any fetch failure resolves to `null`; callers decide from the auth-store
 * snapshot instead (see `hasAssistantAccess`). A 401 additionally clears
 * the auth store inside the api client, so the caller's unauthenticated
 * redirect takes over.
 */
export async function fetchAssistantAccessUser(): Promise<User | null> {
  try {
    return await api.get<User>("/users/me");
  } catch {
    return null;
  }
}

/**
 * The complete /assistant* beforeLoad decision, extracted pure so the
 * ordering-sensitive paths are unit-testable. The router maps the returned
 * decision to side effects (login redirect vs `throw redirect`).
 *
 * `stay` re-runs (search-only updates like `?c=` conversation switches,
 * invalidations) never fetch: entry was already server-verified. They still
 * read the settled store snapshot so a session that died or a flag revoked
 * mid-session evicts on the next in-surface navigation instead of lingering.
 *
 * `enter`/`preload` re-verify against the server. The response is applied to
 * the store ONLY if the session that issued it is still current: a 401 on any
 * concurrent request clears the session mid-await (`setUser(null)` in the api
 * client), and a late `/users/me` success must not resurrect it — sign-out
 * wins over stale data. Same rule if a different user signed in mid-await.
 */
export async function resolveAssistantEntry(opts: {
  readonly cause: "preload" | "enter" | "stay";
  readonly getAuth: () => AuthSnapshot;
  readonly applyUser: (user: User) => void;
  readonly fetchUser?: () => Promise<User | null>;
}): Promise<AssistantEntryDecision> {
  const { cause, getAuth, applyUser } = opts;
  const fetchUser = opts.fetchUser ?? fetchAssistantAccessUser;

  if (cause === "stay") {
    const auth = getAuth();
    if (auth.isLoading) {
      return "allow";
    }
    if (!auth.isAuthenticated) {
      return "redirect-login";
    }
    if (auth.user !== null && !hasAssistantAccess(null, auth.user)) {
      return "redirect-dashboard";
    }
    return "allow";
  }

  const pre = getAuth();
  if (!pre.isAuthenticated && !pre.isLoading) {
    return "redirect-login";
  }

  const fetched = await fetchUser();
  const post = getAuth();

  // Sign-out wins: whatever cleared the session mid-await (a 401 on any
  // concurrent request, an explicit logout) must not be undone by this late
  // response. The login redirect carries return_to, so a session that
  // recovers lands back here.
  if (!post.isAuthenticated) {
    return "redirect-login";
  }

  const sameSession =
    fetched !== null && (post.user === null || post.user.id === fetched.id);
  if (fetched && sameSession) {
    applyUser(fetched);
  }

  return hasAssistantAccess(sameSession ? fetched : null, getAuth().user)
    ? "allow"
    : "redirect-dashboard";
}
