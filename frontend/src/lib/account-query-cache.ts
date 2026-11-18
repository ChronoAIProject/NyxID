import { useAuthStore } from "@/stores/auth-store";
import { QueryClient, type QueryClientConfig } from "@tanstack/react-query";
import {
  AccountChangedError,
  captureAccountOperation,
  getAssistantIdentityRevision,
  isAccountAbort,
  isAccountEvidencePending,
  subscribeAccountEvidence,
  subscribeAssistantIdentity,
} from "./assistant/identity";

/** Bind every query/mutation to the account that admitted it, including unscoped keys. */
export function bindAccountQueryClient(client: QueryClient): () => void {
  const revisions = new WeakMap<object, number>();
  const operations = new WeakMap<object, ReturnType<typeof captureAccountOperation>>();
  const wrapped = new WeakSet<object>();
  const stopMutations = client.getMutationCache().subscribe(({ mutation }) => {
    if (!mutation) return;
    if (!revisions.has(mutation)) {
      revisions.set(mutation, getAssistantIdentityRevision());
      operations.set(mutation, captureAccountOperation());
    }
    const current = () =>
      mutation.options.meta?.identityTransition === true ||
      revisions.get(mutation) === getAssistantIdentityRevision();
    const options = { ...mutation.options };
    // Observer option updates must retain the fence too. A late onError rollback
    // is as account-sensitive as onSuccess/setQueryData.
    for (const key of [
      "onMutate",
      "onSuccess",
      "onError",
      "onSettled",
    ] as const) {
      const callback = options[key];
      if (!callback || wrapped.has(callback)) continue;
      const guarded = (...args: Parameters<NonNullable<typeof callback>>) => {
        if (key === "onError" && isAccountAbort(args[0])) return;
        if (key === "onSettled" && isAccountAbort(args[1])) return;
        if (current()) return Reflect.apply(callback, undefined, args);
      };
      wrapped.add(guarded);
      Object.assign(options, { [key]: guarded });
    }
    const fn = options.mutationFn;
    if (fn && !wrapped.has(fn)) {
      options.mutationFn = async (...args) => {
        if (!current()) throw new AccountChangedError();
        // Capture before onMutate/queue waits; evidence is a dispatch fence only.
        if (mutation.options.meta?.identityTransition !== true) operations.get(mutation)!.assertCurrent();
        const result = await fn(...args);
        if (!current()) throw new AccountChangedError();
        return result;
      };
      wrapped.add(options.mutationFn);
    }
    mutation.setOptions(options);
  });
  const stopIdentity = subscribeAssistantIdentity((_next, previous) => {
    // Cancellation reverts queries to their previous data. A departing account
    // removes everything; first sign-in resets anonymous queries in place so
    // still-mounted observers stay attached to the retained client.
    void client.cancelQueries();
    if (previous !== null) client.clear();
    else {
      client.getMutationCache().clear();
      void client.resetQueries();
    }
  });
  return () => {
    stopMutations();
    stopIdentity();
  };
}

/** Retire clients so a callback already awaiting inside onSuccess cannot reach B. */
export function createAccountQueryScope(
  config: QueryClientConfig,
  onClient?: (client: QueryClient) => () => void,
) {
  let client = new QueryClient(config);
  let stopBinding = bindAccountQueryClient(client);
  let stopClient = onClient?.(client);
  const stopIdentity = subscribeAssistantIdentity((_next, previous) => {
    if (previous === null) return;
    void client.cancelQueries();
    client.clear();
    stopBinding();
    stopClient?.();
    client = new QueryClient(config);
    stopBinding = bindAccountQueryClient(client);
    stopClient = onClient?.(client);
  });
  const stopEvidence = subscribeAccountEvidence(() => {
    if (isAccountEvidencePending()) return;
    const user = useAuthStore.getState().user;
    // An unchanged verifier profile need not publish a new store reference.
    // Refill an initial query refused during evidence without another request.
    if (user) client.setQueryData(["user", "me"], user);
  });
  return {
    getClient: () => client,
    dispose: () => {
      stopEvidence();
      stopIdentity();
      stopBinding();
      stopClient?.();
      client.clear();
    },
  };
}
