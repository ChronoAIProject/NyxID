import { useEffect } from "react";
import { AppState } from "react-native";
import {
  useMutation,
  useIsMutating,
  useQuery,
  useQueryClient,
  type QueryClient,
} from "@tanstack/react-query";
import { getAuthAccountId } from "../../lib/auth/authAccount";
import { authSessionGeneration } from "../../lib/auth/sessionStore";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { quickAccessApi } from "../../lib/api/quickAccessApi";
import {
  QUICK_ACCESS_DEFAULT_STATE,
  type QuickAccessItem,
  type QuickAccessState,
} from "../../lib/api/quickAccessSchema";
import {
  quickAccessRetryDelay,
  refetchIfDue,
  retryQuickAccessRead,
} from "./quickAccessReadRetry";

/** One account's list; a sign-in under another account never reads it. */
export const quickAccessKey = (epoch: number) =>
  ["account", "quick-access", epoch] as const;
/** Every change to the list (save or reset), across screens. */
export const QUICK_ACCESS_MUTATION_KEY = ["account", "quick-access", "change"] as const;
export const QUICK_ACCESS_STALE_MS = 60_000;

/**
 * The read for one account. It recovers on its own: transient failures (the
 * launch token-refresh race, no connection, server errors) are retried with
 * backoff, and a list that could not load is read again when the connection
 * returns (TanStack's online state follows NetInfo; see App.tsx). After a
 * sign-out or account switch the key changes; this account's read never runs
 * or retries under the next session.
 */
export function quickAccessQueryOptions(epoch: number) {
  return {
    queryKey: quickAccessKey(epoch),
    queryFn: ({ signal }: { signal: AbortSignal }) =>
      epoch === getAuthEpoch()
        ? quickAccessApi.get(signal)
        : Promise.reject(new Error("auth_session_changed")),
    staleTime: QUICK_ACCESS_STALE_MS,
    retry: (failureCount: number, error: unknown) =>
      epoch === getAuthEpoch() && retryQuickAccessRead(failureCount, error),
    retryDelay: quickAccessRetryDelay,
    refetchOnReconnect: true,
  };
}

/**
 * Back in front: read the list again if its last read failed or went stale.
 * A change in flight keeps the last word.
 */
export function refreshQuickAccessOnForeground(
  client: QueryClient,
  epoch: number,
  now = Date.now(),
): boolean {
  if (epoch !== getAuthEpoch()) return false;
  if (client.isMutating({ mutationKey: QUICK_ACCESS_MUTATION_KEY }) > 0)
    return false;
  return refetchIfDue(client, quickAccessKey(epoch), QUICK_ACCESS_STALE_MS, now);
}

/** A change: a new ordered list, or back to the default. */
export type QuickAccessChange =
  | { items: readonly QuickAccessItem[] }
  | { reset: true };

type ActionAuthority = { account: string | null; epoch: number; generation: number };
type BoundChange = QuickAccessChange & { authority: ActionAuthority };
function captureAuthority(): ActionAuthority {
  return { account: getAuthAccountId(), epoch: getAuthEpoch(), generation: authSessionGeneration() };
}
function ownsAction(authority: ActionAuthority) {
  return authority.account === getAuthAccountId() && authority.epoch === getAuthEpoch() && authority.generation === authSessionGeneration();
}
function assertAction(authority: ActionAuthority) {
  if (!ownsAction(authority)) throw Object.assign(new Error("auth_session_changed"), { name: "AbortError" });
}

/** What the list looks like the moment a change is made. */
export function optimisticQuickAccess(change: QuickAccessChange): QuickAccessState {
  return "reset" in change
    ? QUICK_ACCESS_DEFAULT_STATE
    : { items: [...change.items], customized: true };
}

/**
 * Optimistic update with rollback: the change shows at once (the rail and
 * Settings read the same cache), a failure restores what was there, and the
 * server's answer (or a refetch after a failure) has the last word.
 */
export function quickAccessMutationHandlers(
  client: QueryClient,
  key: ReturnType<typeof quickAccessKey>,
) {
  const current = (change?: QuickAccessChange & { authority?: ActionAuthority }) =>
    key[2] === getAuthEpoch() && (!change?.authority || ownsAction(change.authority));
  return {
    onMutate: async (change: QuickAccessChange & { authority?: ActionAuthority }) => {
      if (!current(change)) throw Object.assign(new Error("auth_session_changed"), { name: "AbortError" });
      await client.cancelQueries({ queryKey: key });
      if (!current(change)) throw Object.assign(new Error("auth_session_changed"), { name: "AbortError" });
      const previous = client.getQueryData<QuickAccessState>(key);
      client.setQueryData<QuickAccessState>(key, optimisticQuickAccess(change));
      return { previous };
    },
    onError: (
      _error: unknown,
      change: QuickAccessChange & { authority?: ActionAuthority },
      context: { previous?: QuickAccessState } | undefined,
    ) => {
      if (!current(change)) return;
      if (context?.previous) client.setQueryData(key, context.previous);
      else client.removeQueries({ queryKey: key, exact: true });
    },
    onSuccess: (saved: QuickAccessState, change?: QuickAccessChange & { authority?: ActionAuthority }) => {
      if (!current(change)) return;
      client.setQueryData(key, saved);
    },
    onSettled: (
      _saved: QuickAccessState | undefined,
      error: unknown,
      change?: QuickAccessChange & { authority?: ActionAuthority },
    ) => {
      if (!current(change)) return;
      if (error) void client.invalidateQueries({ queryKey: key });
    },
  };
}

/**
 * The Duo's quick-access list, saved to the account. Until it loads (and if
 * it cannot), the default list shows, so the rail never starts empty.
 * Changes apply optimistically, one at a time, in the order they were made.
 */
export function useQuickAccess({ enabled = true }: { enabled?: boolean } = {}) {
  const client = useQueryClient();
  const epoch = getAuthEpoch();
  const key = quickAccessKey(epoch);
  const query = useQuery({ ...quickAccessQueryOptions(epoch), enabled });
  useEffect(() => {
    if (!enabled) return;
    const subscription = AppState.addEventListener("change", (state) => {
      if (state === "active") refreshQuickAccessOnForeground(client, epoch);
    });
    return () => subscription.remove();
  }, [client, enabled, epoch]);
  const mutation = useMutation({
    mutationKey: QUICK_ACCESS_MUTATION_KEY,
    mutationFn: (change: BoundChange) => {
      const guard = () => assertAction(change.authority);
      guard();
      return "reset" in change ? quickAccessApi.reset(guard) : quickAccessApi.save(change.items, guard);
    },
    // One account list: changes run in order, never racing each other.
    scope: { id: "account-quick-access" },
    ...quickAccessMutationHandlers(client, key),
  });
  return {
    state: query.data ?? QUICK_ACCESS_DEFAULT_STATE,
    loaded: query.data !== undefined,
    loadError: query.error,
    // Account-wide pending state survives closing/reopening the editor. Its
    // replacement observer must not edit over an older optimistic response.
    saving: useIsMutating({ mutationKey: QUICK_ACCESS_MUTATION_KEY, exact: true }) > 0,
    saveError: mutation.error,
    save: (items: readonly QuickAccessItem[], complete?: () => void) => mutation.mutate({ items, authority: captureAuthority() }, { onSettled: complete }),
    reset: (complete?: () => void) => mutation.mutate({ reset: true, authority: captureAuthority() }, { onSettled: complete }),
    clearSaveError: () => mutation.reset(),
    retry: () => void query.refetch(),
  };
}
