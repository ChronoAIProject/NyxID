import { useFeature } from "@/hooks/use-feature-flag";
import { FEATURE_FLAG, userHasFeature } from "@/lib/feature-flags";
import { useAuthStore } from "@/stores/auth-store";

/** Personal admission only; ongoing Machine management stays available. */
export function useRemoteMachineAdmission(): boolean {
  const enabled = useFeature(FEATURE_FLAG.REMOTE_MACHINES);
  const ready = useAuthStore(
    (state) =>
      state.isAuthenticated && !state.isLoading && Boolean(state.user?.id),
  );
  return ready && enabled;
}

/** Recheck the captured actor at dispatch and across asynchronous transport boundaries. */
export function assertRemoteMachineAdmission(owner: string | undefined): void {
  const state = useAuthStore.getState();
  if (!owner || state.user?.id !== owner)
    throw new Error(
      "Your account changed. Reload before managing this Machine.",
    );
  if (
    !state.isAuthenticated ||
    state.isLoading ||
    !userHasFeature(state.user, FEATURE_FLAG.REMOTE_MACHINES)
  )
    throw new Error("Remote Machines are not enabled for this account.");
}
