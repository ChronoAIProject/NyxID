import type { User } from "@/types/api";
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
