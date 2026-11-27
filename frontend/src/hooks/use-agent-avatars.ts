import { FEATURE_FLAG, userHasFeature } from "@/lib/feature-flags";
import { useAuthStore } from "@/stores/auth-store";

/** Whether personalised avatars are on for the signed-in person (person,
 * org and global rollout, as resolved on `/users/me`). No query: avatars
 * render in many places, some outside a query client. */
export function useAgentAvatarsEnabled(): boolean {
  return useAuthStore((state) => userHasFeature(state.user, FEATURE_FLAG.ASSISTANT_AVATARS));
}
