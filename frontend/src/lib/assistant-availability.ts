import type { User } from "@/types/api";
import { api } from "@/lib/api-client";
import { FEATURE_FLAG } from "@/lib/feature-flags";

type AssistantAvailabilityUser = Pick<User, "capabilities"> | null;

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
