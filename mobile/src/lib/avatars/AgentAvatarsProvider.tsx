import type { ReactNode } from "react";
import { useQuery } from "@tanstack/react-query";
import { getCurrentUserProfileRequest } from "../api/http";
import { useAuthSession } from "../../features/auth/AuthSessionContext";
import { AGENT_AVATARS_FLAG, AgentAvatarsFlag } from "./agentAvatarsFlag";

/**
 * Supplies whether `assistant:avatars` is on for the signed-in person
 * (person, org and global rollout, as resolved on `/users/me`), from the
 * profile the workspace already caches. Avatars render in many places, some
 * outside a query, so they read context; without this provider every avatar
 * keeps today's look.
 */
export function AgentAvatarsProvider({ children }: { children: ReactNode }) {
  // Only a signed-in person's profile is requested.
  const enabled = useAuthSession().isAuthenticated;
  const profile = useQuery({
    queryKey: ["account", "profile"],
    queryFn: ({ signal }) => getCurrentUserProfileRequest(signal),
    enabled,
    staleTime: 60_000,
    retry: false,
  });
  const on =
    enabled &&
    (profile.data?.capabilities?.enabled_features?.includes(
      AGENT_AVATARS_FLAG,
    ) ??
      false);
  return <AgentAvatarsFlag value={on}>{children}</AgentAvatarsFlag>;
}
