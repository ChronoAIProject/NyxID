import type { User } from "@/types/api";
import { api } from "@/lib/api-client";
import { FEATURE_FLAG } from "@/lib/feature-flags";

type AssistantAvailabilityUser = Pick<User, "capabilities"> | null;

export function shouldRedirectFromAssistant(auth: {
  readonly isLoading: boolean;
  readonly user: AssistantAvailabilityUser;
}): boolean {
  return (
    !auth.isLoading &&
    !(auth.user?.capabilities?.enabled_features ?? []).includes(
      FEATURE_FLAG.AI_ASSISTANT,
    )
  );
}

/**
 * Server-verified flag lookup for the /assistant* route guards: re-fetches
 * `/users/me` so route entry reflects the backend's current flag resolution
 * instead of the boot-time auth-store snapshot (a flag disabled mid-session
 * must lock the route on the next navigation, not the next hard reload).
 *
 * Fail-closed: any fetch failure resolves to `null`, which callers treat as
 * flag-off. A 401 additionally clears the auth store inside the api client,
 * so the caller's unauthenticated redirect takes over.
 */
export async function fetchAssistantAccessUser(): Promise<User | null> {
  try {
    return await api.get<User>("/users/me");
  } catch {
    return null;
  }
}
