import { QueryClient, type QueryClientConfig } from "@tanstack/react-query";
import {
  getAssistantIdentityRevision,
  subscribeAssistantIdentity,
} from "./assistant/identity";

/** Bind every query/mutation to the account that admitted it, including unscoped keys. */
export function bindAccountQueryClient(client: QueryClient): () => void {
  const revisions = new WeakMap<object, number>();
  const wrapped = new WeakSet<object>();
  const stopMutations = client.getMutationCache().subscribe(({ mutation }) => {
    if (!mutation) return;
    if (!revisions.has(mutation))
      revisions.set(mutation, getAssistantIdentityRevision());
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
        if (current()) return Reflect.apply(callback, undefined, args);
      };
      wrapped.add(guarded);
      Object.assign(options, { [key]: guarded });
    }
    const fn = options.mutationFn;
    if (fn && !wrapped.has(fn)) {
      options.mutationFn = async (...args) => {
        if (!current()) throw new DOMException("Account changed", "AbortError");
        const result = await fn(...args);
        if (!current()) throw new DOMException("Account changed", "AbortError");
        return result;
      };
      wrapped.add(options.mutationFn);
    }
    mutation.setOptions(options);
  });
  const stopIdentity = subscribeAssistantIdentity(() => {
    // Cancellation reverts queries to their previous data; remove after cancelling.
    void client.cancelQueries();
    client.clear();
  });
  return () => {
    stopMutations();
    stopIdentity();
  };
}

/** Retire clients so a callback already awaiting inside onSuccess cannot reach B. */
export function createAccountQueryScope(config: QueryClientConfig) {
  let client = new QueryClient(config);
  let stopBinding = bindAccountQueryClient(client);
  const stopIdentity = subscribeAssistantIdentity(() => {
    void client.cancelQueries();
    client.clear();
    stopBinding();
    client = new QueryClient(config);
    stopBinding = bindAccountQueryClient(client);
  });
  return {
    getClient: () => client,
    dispose: () => {
      stopIdentity();
      stopBinding();
      client.clear();
    },
  };
}
