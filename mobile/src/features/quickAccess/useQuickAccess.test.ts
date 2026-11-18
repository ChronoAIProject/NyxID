import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath, URL } from "node:url";
import {
  MutationObserver,
  QueryClient,
  QueryObserver,
} from "@tanstack/query-core";
import { hookHarness } from "../assistant/assistantHookTestHarness";
import { ApiError } from "../../lib/api/ApiError";
import {
  QUICK_ACCESS_DEFAULT_STATE,
  type QuickAccessState,
} from "../../lib/api/quickAccessSchema";
import {
  QUICK_ACCESS_READ_RETRIES,
  quickAccessRetryDelay,
  retryQuickAccessRead,
} from "./quickAccessReadRetry";

const AGENT = "6f9619ff-8b86-d011-b42d-00c04fc964ff";
const saved: QuickAccessState = {
  customized: true,
  items: [{ kind: "agent", id: AGENT }],
};
// The harness evaluates modules in their own realm: compare objects as JSON
// with sorted keys.
const canonical = (value: unknown) =>
  JSON.stringify(value, (_key, entry) =>
    entry && typeof entry === "object" && !Array.isArray(entry)
      ? Object.fromEntries(Object.entries(entry).sort(([a], [b]) => a.localeCompare(b)))
      : entry,
  );
const same = (actual: unknown, expected: unknown, message = "") =>
  assert.equal(canonical(actual), canonical(expected), message);

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}

const apiError = (statusCode: number) =>
  new ApiError({
    errorKey: "synthetic",
    errorCode: 1000,
    statusCode,
    message: `synthetic ${statusCode}`,
  });

/**
 * The hook module with its own API, auth epoch and AppState stubbed. `epoch`
 * and `get` can change between calls; AppState listeners are collected.
 */
function load(
  extra: Record<string, unknown> = {},
  options: { get?: () => Promise<unknown> } = {},
) {
  const env = {
    epoch: 7,
    reads: 0,
    get: options.get ?? (async () => saved as unknown),
    appState: [] as ((state: string) => void)[],
  };
  const h = hookHarness<any>({
    "../../lib/auth/authEpoch": { getAuthEpoch: () => env.epoch },
    "../../lib/api/quickAccessApi": {
      quickAccessApi: {
        get: () => {
          env.reads++;
          return env.get();
        },
      },
    },
    "react-native": {
      AppState: {
        currentState: "active",
        addEventListener: (_event: string, listener: (state: string) => void) => {
          env.appState.push(listener);
          return {
            remove: () => {
              env.appState = env.appState.filter((entry) => entry !== listener);
            },
          };
        },
      },
    },
    ...extra,
  });
  const module = h.load(
    fileURLToPath(new URL("./useQuickAccess.ts", import.meta.url)),
  ) as typeof import("./useQuickAccess");
  return Object.assign(module, { h, env });
}

/** Wait until the observer's query settles (success, or error with no retry left). */
type Settling = {
  fetchStatus: string;
  status: string;
  data?: unknown;
};
function settled(observer: {
  getCurrentResult(): Settling;
  subscribe(listener: () => void): () => void;
}) {
  return new Promise<Settling>((resolve) => {
    const check = () => {
      const result = observer.getCurrentResult();
      if (result.fetchStatus === "idle" && result.status !== "pending") {
        unsubscribe();
        resolve(result);
      }
    };
    const unsubscribe = observer.subscribe(check);
    check();
  });
}

test("a change shows at once, the server's answer replaces it, and a failure puts the previous list back", async () => {
  const module = load();
  const client = new QueryClient();
  const key = module.quickAccessKey(7);
  client.setQueryData(key, saved);
  const handlers = module.quickAccessMutationHandlers(client as any, key);
  const calls: string[] = [];
  const pending: ReturnType<typeof deferred<QuickAccessState>>[] = [];
  const observer = new MutationObserver(client, {
    mutationFn: (change: any) => {
      calls.push("reset" in change ? "reset" : `save:${change.items.length}`);
      const next = deferred<QuickAccessState>();
      pending.push(next);
      return next.promise;
    },
    scope: { id: "account-quick-access" },
    ...(handlers as any),
  });

  // Optimistic: the new list is in the cache before the server answers.
  const next = [...saved.items, { kind: "destination", id: "machines" } as const];
  const first = observer.mutate({ items: next });
  await new Promise((resolve) => setImmediate(resolve));
  same(client.getQueryData(key), { items: next, customized: true });
  const answer: QuickAccessState = { customized: true, items: next };
  pending[0]!.resolve(answer);
  await first;
  same(client.getQueryData(key), answer);
  assert.equal(client.getQueryState(key)?.isInvalidated, false, "a success needs no refetch");

  // Rollback: a failed save restores exactly what was shown before it, and
  // the list is refetched so the server has the last word.
  const failing = observer.mutate({ items: [] }).catch(() => "failed");
  await new Promise((resolve) => setImmediate(resolve));
  same(client.getQueryData(key), { items: [], customized: true });
  pending[1]!.reject(new Error("offline"));
  assert.equal(await failing, "failed");
  same(client.getQueryData(key), answer);
  assert.equal(client.getQueryState(key)?.isInvalidated, true);

  // Reset shows the default at once.
  const reset = observer.mutate({ reset: true });
  await new Promise((resolve) => setImmediate(resolve));
  same(client.getQueryData(key), QUICK_ACCESS_DEFAULT_STATE);
  pending[2]!.resolve(QUICK_ACCESS_DEFAULT_STATE);
  await reset;
  assert.deepEqual(calls, ["save:2", "save:0", "reset"]);
});

test("changes made in a row run one at a time, in order (one account list)", async () => {
  const module = load();
  const client = new QueryClient();
  const key = module.quickAccessKey(7);
  client.setQueryData(key, saved);
  const handlers = module.quickAccessMutationHandlers(client as any, key);
  const started: string[] = [];
  const pending: ReturnType<typeof deferred<QuickAccessState>>[] = [];
  const options = {
    mutationFn: (change: any) => {
      started.push(change.items.map((item: any) => item.id).join(","));
      const next = deferred<QuickAccessState>();
      pending.push(next);
      return next.promise;
    },
    scope: { id: "account-quick-access" },
    ...(handlers as any),
  };
  // Two observers (two screens: Settings and the rail share the scope).
  const a = new MutationObserver<unknown, unknown, any>(client, options).mutate({
    items: [{ kind: "destination", id: "plugins" }],
  });
  const b = new MutationObserver<unknown, unknown, any>(client, options).mutate({
    items: [{ kind: "destination", id: "activity" }],
  });
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(started, ["plugins"], "the second waits for the first");
  pending[0]!.resolve({ customized: true, items: [{ kind: "destination", id: "plugins" }] });
  await a;
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(started, ["plugins", "activity"]);
  pending[1]!.resolve({ customized: true, items: [{ kind: "destination", id: "activity" }] });
  await b;
  same(client.getQueryData(key), {
    customized: true,
    items: [{ kind: "destination", id: "activity" }],
  });
});

test("a failed first save with nothing cached drops the optimistic list (back to the default until it loads)", async () => {
  const module = load();
  const client = new QueryClient();
  const key = module.quickAccessKey(7);
  const handlers = module.quickAccessMutationHandlers(client as any, key);
  const context = await handlers.onMutate({ items: saved.items });
  same(client.getQueryData(key), saved);
  handlers.onError(new Error("offline"), { items: saved.items }, context);
  assert.equal(client.getQueryData(key), undefined);
  same(module.optimisticQuickAccess({ reset: true }), QUICK_ACCESS_DEFAULT_STATE);
});

test("the hook reads one account's list (default until loaded), saves and resets through one scoped mutation", () => {
  const queries: any[] = [];
  const mutations: any[] = [];
  const mutated: unknown[] = [];
  let data: QuickAccessState | undefined;
  const module = load({
    "@tanstack/react-query": {
      useQueryClient: () => new QueryClient(),
      useQuery: (options: any) => {
        queries.push(options);
        return { data, error: null, refetch: () => {} };
      },
      useMutation: (options: any) => {
        mutations.push(options);
        return {
          mutate: (change: unknown) => mutated.push(change),
          isPending: false,
          error: null,
          reset: () => {},
        };
      },
    },
  });
  const { h, env } = module;
  h.mount(() => module.useQuickAccess());
  same(h.value().state, QUICK_ACCESS_DEFAULT_STATE, "the rail never starts empty");
  assert.equal(h.value().loaded, false);
  const read = queries.at(-1);
  same(read.queryKey, ["account", "quick-access", 7]);
  // Transient failures retry with backoff and reconnects refetch.
  assert.equal(typeof read.retry, "function");
  assert.equal(read.retryDelay, quickAccessRetryDelay);
  assert.equal(read.refetchOnReconnect, true);
  assert.equal(read.staleTime, 60_000);
  same(mutations.at(-1).scope, { id: "account-quick-access" });
  same(mutations.at(-1).mutationKey, module.QUICK_ACCESS_MUTATION_KEY);
  // One foreground listener while mounted.
  assert.equal(env.appState.length, 1);
  data = saved;
  h.render();
  same(h.value().state, saved);
  assert.equal(h.value().loaded, true);
  h.value().save(saved.items);
  h.value().reset();
  same(mutated, [{ items: saved.items }, { reset: true }]);
  h.unmount();
  assert.equal(env.appState.length, 0, "the listener goes with the screen");
  h.mount(() => module.useQuickAccess({ enabled: false }));
  assert.equal(queries.at(-1).enabled, false);
  assert.equal(env.appState.length, 0, "a disabled list does not listen");
  h.unmount();
});

test("transient read failures retry with backoff; validation refusals and an unreadable answer are final", () => {
  const sessionChanged = new Error("auth_session_changed");
  for (const status of [0, 401, 408, 429, 500, 502, 503, 504])
    assert.equal(retryQuickAccessRead(0, apiError(status)), true, `${status}`);
  assert.equal(retryQuickAccessRead(0, sessionChanged), true);
  for (const status of [400, 403, 404, 409, 422])
    assert.equal(retryQuickAccessRead(0, apiError(status)), false, `${status}`);
  assert.equal(retryQuickAccessRead(0, new Error("NyxID returned an unexpected response.")), false);
  assert.equal(
    retryQuickAccessRead(0, Object.assign(new Error("cancelled"), { name: "AbortError" })),
    false,
  );
  // Bounded: three repeats after the first failure.
  assert.equal(retryQuickAccessRead(QUICK_ACCESS_READ_RETRIES - 1, apiError(503)), true);
  assert.equal(retryQuickAccessRead(QUICK_ACCESS_READ_RETRIES, apiError(503)), false);
  assert.deepEqual(
    [0, 1, 2, 3].map((count) => quickAccessRetryDelay(count, apiError(503))),
    [1000, 2000, 4000, 4000],
  );
  const limited = new ApiError({
    errorKey: "rate_limited",
    errorCode: 1000,
    statusCode: 429,
    message: "slow down",
    retryAfterMs: 6000,
  });
  assert.equal(quickAccessRetryDelay(0, limited), 6000, "Retry-After is honoured");
  assert.equal(
    quickAccessRetryDelay(0, Object.assign(limited, { retryAfterMs: 60_000 })),
    8000,
    "but never holds the rail for long",
  );
});

test("a read that fails during the launch token refresh is retried and loads the saved list", async () => {
  let attempt = 0;
  const module = load({}, {
    // The first read carries the expired token; the next has the new one.
    get: async () => {
      attempt++;
      if (attempt === 1) throw apiError(401);
      if (attempt === 2) throw new Error("auth_session_changed");
      return saved;
    },
  });
  const client = new QueryClient();
  const observer = new QueryObserver(client, {
    ...module.quickAccessQueryOptions(7),
    retryDelay: 1,
  });
  const result = await settled(observer);
  assert.equal(result.status, "success");
  same(result.data, saved);
  assert.equal(module.env.reads, 3);
  same(client.getQueryData(module.quickAccessKey(7)), saved);
});

test("a refusal is not retried, and an account switch stops the old account's retries", async () => {
  const refused = load({}, { get: async () => { throw apiError(400); } });
  const client = new QueryClient();
  const first = await settled(
    new QueryObserver(client, { ...refused.quickAccessQueryOptions(7), retryDelay: 1 }),
  );
  assert.equal(first.status, "error");
  assert.equal(refused.env.reads, 1);

  const switched = load({}, {
    get: async () => {
      switched.env.epoch = 8; // signed out / another account while this read failed
      throw apiError(503);
    },
  });
  const other = new QueryClient();
  const result = await settled(
    new QueryObserver(other, { ...switched.quickAccessQueryOptions(7), retryDelay: 1 }),
  );
  assert.equal(result.status, "error");
  assert.equal(switched.env.reads, 1, "no retry under the next session");
  // And a read keyed to the old account never runs under the new one.
  await other.prefetchQuery({ ...switched.quickAccessQueryOptions(7), retry: false });
  assert.equal(switched.env.reads, 1);
});

test("back in front, a list that failed to load (or went stale) is read again; a fresh one or a change in flight is left alone", async () => {
  let failing = true;
  const module = load({}, {
    get: async () => {
      if (failing) throw apiError(503);
      return saved;
    },
  });
  const client = new QueryClient();
  const key = module.quickAccessKey(7);
  // Every retry exhausted while the device was offline.
  await client.prefetchQuery({ ...module.quickAccessQueryOptions(7), retry: false });
  assert.equal(client.getQueryState(key)?.status, "error");
  failing = false;
  assert.equal(module.refreshQuickAccessOnForeground(client, 7), true);
  await new Promise((resolve) => setImmediate(resolve));
  same(client.getQueryData(key), saved);
  const reads = module.env.reads;
  // Fresh: nothing to do.
  assert.equal(module.refreshQuickAccessOnForeground(client, 7), false);
  // Stale after a minute away.
  const later = Date.now() + module.QUICK_ACCESS_STALE_MS;
  assert.equal(module.refreshQuickAccessOnForeground(client, 7, later), true);
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(module.env.reads, reads + 1);
  // A save in flight keeps the last word.
  const pending = new Promise(() => {});
  void new MutationObserver(client, {
    mutationKey: module.QUICK_ACCESS_MUTATION_KEY,
    mutationFn: () => pending,
  }).mutate(undefined as never).catch(() => {});
  assert.equal(
    module.refreshQuickAccessOnForeground(client, 7, later + module.QUICK_ACCESS_STALE_MS),
    false,
  );
  // Another account's list is never refreshed from this one.
  module.env.epoch = 8;
  assert.equal(module.refreshQuickAccessOnForeground(client, 7, later * 2), false);
});

test("the hook's foreground listener refreshes the mounted account's failed list", async () => {
  let failing = true;
  const client = new QueryClient();
  const module = load(
    {
      "@tanstack/react-query": {
        useQueryClient: () => client,
        useQuery: () => ({ data: undefined, error: null, refetch: () => {} }),
        useMutation: () => ({
          mutate: () => {},
          isPending: false,
          error: null,
          reset: () => {},
        }),
      },
    },
    {
      get: async () => {
        if (failing) throw apiError(0);
        return saved;
      },
    },
  );
  const key = module.quickAccessKey(7);
  await client.prefetchQuery({ ...module.quickAccessQueryOptions(7), retry: false });
  module.h.mount(() => module.useQuickAccess());
  failing = false;
  // Background and inactive transitions do nothing.
  module.env.appState.forEach((listener) => listener("background"));
  module.env.appState.forEach((listener) => listener("inactive"));
  assert.equal(module.env.reads, 1);
  module.env.appState.forEach((listener) => listener("active"));
  await new Promise((resolve) => setImmediate(resolve));
  same(client.getQueryData(key), saved);
  assert.equal(module.env.reads, 2);
  module.h.unmount();
});
