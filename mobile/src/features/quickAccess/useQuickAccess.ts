import { useEffect } from "react";
import { AppState } from "react-native";
import {
  useMutation,
  useIsMutating,
  useQuery,
  useQueryClient,
  type QueryClient,
} from "@tanstack/react-query";
import { isApiError } from "../../lib/api/ApiError";
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
const writeKey = (epoch: number) => ['account', 'quick-access-write', epoch] as const;
type WriteState = {revision: number; pending: number};
type WriteTicket = {finished: boolean; owner: object};
type MutationContext = {previous?: QuickAccessState; ticket?: WriteTicket};
const writeState = (client: QueryClient, epoch: number): WriteState =>
  client.getQueryData<WriteState>(writeKey(epoch)) ?? {revision: 0, pending: 0};
function admitWrite(client: QueryClient, epoch: number): WriteTicket {
  // The owner may close every editor/rail while a request is still pending.
  // Retain the account fence until the auth lifecycle clears the client.
  client.setQueryDefaults(writeKey(epoch), {gcTime: Infinity});
  const state = writeState(client, epoch);
  client.setQueryData(writeKey(epoch), {revision: state.revision + 1, pending: state.pending + 1});
  return {finished: false, owner: client.getQueryCache().find({queryKey: writeKey(epoch), exact: true})!};
}
const ownsWriteCache = (client: QueryClient, epoch: number, ticket?: WriteTicket) =>
  !!ticket && client.getQueryCache().find({queryKey: writeKey(epoch), exact: true}) === ticket.owner;
function finishWrite(client: QueryClient, epoch: number, ticket?: WriteTicket) {
  if (!ticket || ticket.finished) return;
  ticket.finished = true;
  // A completed sign-out/replacement clears the client. Never recreate its
  // retired metadata, or decrement a new login's ticket under the same key.
  if (!ownsWriteCache(client, epoch, ticket)) return;
  const state = writeState(client, epoch);
  client.setQueryData(writeKey(epoch), {revision: state.revision + 1, pending: Math.max(0, state.pending - 1)});
}
const conflictKey = (epoch: number) => ['account', 'quick-access-conflict', epoch] as const;
type ConflictRecovery = {phase: 'required' | 'ready'};
const needsRecovery = (client: QueryClient, epoch: number) =>
  client.getQueryData<ConflictRecovery>(conflictKey(epoch))?.phase === 'required';
const conflictError = (error: unknown) => isApiError(error) && error.statusCode === 409
  && (error.errorCode === 1004 || error.errorKey === 'conflict');

/**
 * The read for one account. It recovers on its own: transient failures (the
 * launch token-refresh race, no connection, server errors) are retried with
 * backoff, and a list that could not load is read again when the connection
 * returns (TanStack's online state follows NetInfo; see App.tsx). After a
 * sign-out or account switch the key changes; this account's read never runs
 * or retries under the next session.
 */
export function quickAccessQueryOptions(epoch: number, client?: QueryClient) {
  return {
    queryKey: quickAccessKey(epoch),
    queryFn: async ({ signal }: { signal: AbortSignal }) => {
      if (epoch !== getAuthEpoch()) throw new Error("auth_session_changed");
      const recovery = client?.getQueryData<ConflictRecovery>(conflictKey(epoch));
      const started = client ? writeState(client, epoch) : undefined;
      // A pending write owns the list, including Reset during conflict
      // recovery. A refused write releases this fence before its recovery GET.
      if (client && started!.pending > 0) {
        const cached = client.getQueryData<QuickAccessState>(quickAccessKey(epoch));
        if (cached) return cached;
      }
      const fresh = await quickAccessApi.get(signal);
      if (client && (writeState(client, epoch).revision !== started!.revision || started!.pending > 0)) {
        // A read spanning a mutation cannot hand its old snapshot to TanStack.
        return client.getQueryData<QuickAccessState>(quickAccessKey(epoch)) ?? fresh;
      }
      if (client && !signal.aborted && epoch === getAuthEpoch()
        && recovery?.phase === 'required'
        && client.getQueryData(conflictKey(epoch)) === recovery) {
        // Publish the authoritative list before opening the editor fence.
        client.setQueryData(quickAccessKey(epoch), fresh);
        client.setQueryData<ConflictRecovery>(conflictKey(epoch), {phase: 'ready'});
      }
      return fresh;
    },
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
      if (!('reset' in change) && needsRecovery(client, key[2]))
        throw Object.assign(new Error('quick_access_recovery_required'), {name: 'AbortError'});
      const ticket = admitWrite(client, key[2]);
      if (client.getQueryData<ConflictRecovery>(conflictKey(key[2]))?.phase === 'ready')
        client.setQueryData(conflictKey(key[2]), null);
      try {
        await client.cancelQueries({ queryKey: key });
        if (!current(change)) throw Object.assign(new Error("auth_session_changed"), { name: "AbortError" });
        const previous = client.getQueryData<QuickAccessState>(key);
        client.setQueryData<QuickAccessState>(key, optimisticQuickAccess(change));
        return { previous, ticket };
      } catch (error) {
        finishWrite(client, key[2], ticket);
        throw error;
      }
    },
    onError: (
      _error: unknown,
      change: QuickAccessChange & { authority?: ActionAuthority },
      context: MutationContext | undefined,
    ) => {
      finishWrite(client, key[2], context?.ticket);
      if (!current(change)) return;
      if (conflictError(_error)) client.setQueryData<ConflictRecovery>(conflictKey(key[2]), {phase: 'required'});
      if (_error instanceof Error && _error.name === 'AbortError') return;
      if (context?.previous) client.setQueryData(key, context.previous);
      else client.removeQueries({ queryKey: key, exact: true });
    },
    onSuccess: (saved: QuickAccessState, change?: QuickAccessChange & { authority?: ActionAuthority }, context?: MutationContext) => {
      finishWrite(client, key[2], context?.ticket);
      if (!current(change)) return;
      // Also cancel reads which returned an optimistic cache value while this
      // write was pending, before publishing the authoritative response.
      void client.cancelQueries({queryKey: key});
      client.setQueryData(key, saved);
      if (change && 'reset' in change && needsRecovery(client, key[2]))
        client.setQueryData<ConflictRecovery>(conflictKey(key[2]), {phase: 'ready'});
    },
    onSettled: (
      _saved: QuickAccessState | undefined,
      error: unknown,
      change?: QuickAccessChange & { authority?: ActionAuthority },
      context?: MutationContext,
    ) => {
      finishWrite(client, key[2], context?.ticket);
      if (!current(change)) {
        // A failed durable clear revokes the generation but leaves this exact
        // account/cache alive. Its old response cannot publish; reconcile it
        // with a new read after releasing the admitted write instead.
        if (key[2] === getAuthEpoch()
          && change?.authority?.account === getAuthAccountId()
          && ownsWriteCache(client, key[2], context?.ticket))
          void client.invalidateQueries({queryKey: key});
        return;
      }
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
  // Cache-only account state survives editor/rail unmounts until sign-out
  // clears the QueryClient; an ordinary failed GET never removes the fence.
  const recovery = useQuery<ConflictRecovery | null>({
    queryKey: conflictKey(epoch), queryFn: () => null, enabled: false, gcTime: Infinity,
  });
  const query = useQuery({ ...quickAccessQueryOptions(epoch, client), enabled });
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
      if (!('reset' in change) && needsRecovery(client, epoch))
        throw Object.assign(new Error('quick_access_recovery_required'), {name: 'AbortError'});
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
    recoveryRequired: recovery.data?.phase === 'required',
    recoveryPending: recovery.data?.phase === 'required' && (query.isFetching || !query.error),
    recoveryError: recovery.data?.phase === 'required' ? query.error : null,
    conflictRecovered: recovery.data?.phase === 'ready',
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
