import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath, URL } from "node:url";
import { QueryClient, QueryObserver } from "@tanstack/query-core";
import { hookHarness } from "../assistant/assistantHookTestHarness";
import { ApiError } from "../../lib/api/ApiError";

const AGENT = { id: "6f9619ff-8b86-d011-b42d-00c04fc964ff", name: "researcher" };
const CREW = { id: "nyxg-0123456789abcdef0123456789abcdef", name: "Launch crew" };
// The harness evaluates the hook in its own realm: compare objects as JSON.
const same = (actual: unknown, expected: unknown, message = "") =>
  assert.equal(JSON.stringify(actual), JSON.stringify(expected), message);
const expired = () =>
  new ApiError({
    errorKey: "unauthorized",
    errorCode: 1001,
    statusCode: 401,
    message: "Token expired",
  });

/** The hook with its APIs, auth epoch, AppState and query hooks stubbed. */
function setup({ failFirst = true } = {}) {
  const client = new QueryClient();
  const env = {
    epoch: 7,
    agentReads: 0,
    crewReads: 0,
    failing: failFirst,
    appState: [] as ((state: string) => void)[],
    queries: [] as any[],
  };
  const h = hookHarness<any>({
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
    "@tanstack/react-query": {
      useQueryClient: () => client,
      useQuery: (options: any) => {
        env.queries.push(options);
        return { data: client.getQueryData(options.queryKey) };
      },
    },
    "../../lib/auth/authEpoch": { getAuthEpoch: () => env.epoch },
    "../../lib/api/agentsApi": {
      agentsApi: {
        list: async () => {
          env.agentReads++;
          if (env.failing) throw expired();
          return { agents: [AGENT] };
        },
      },
    },
    "../../lib/api/groupsApi": {
      groupsApi: {
        list: async () => {
          env.crewReads++;
          if (env.failing) throw expired();
          return [CREW];
        },
      },
    },
  });
  const module = h.load(
    fileURLToPath(new URL("./useQuickAccessSources.ts", import.meta.url)),
  ) as typeof import("./useQuickAccessSources");
  return { h, module, client, env };
}

/** Run a captured query's own options to completion on the real client. */
function run(client: QueryClient, options: any) {
  const observer = new QueryObserver(client, { ...options, retryDelay: 1 });
  return new Promise<{ status: string; data?: unknown }>((resolve) => {
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

test("pinned agents and crews share the side menu's lists and recover from a launch auth race", async () => {
  const { h, module, client, env } = setup();
  h.mount(() => module.useQuickAccessSources({ agents: true, crews: true }));
  const [agents, crews] = env.queries.slice(-2);
  same(agents.queryKey, ["workspace", "agents", "drawer", 7]);
  same(crews.queryKey, ["workspace", "groups", 7, "list"]);
  for (const options of [agents, crews]) {
    assert.equal(options.refetchOnReconnect, true);
    assert.equal(options.staleTime, module.QUICK_ACCESS_SOURCES_STALE_MS);
    assert.equal(typeof options.retry, "function");
  }
  // The first read of each carries the expired token; the retry loads it.
  let attempts = 0;
  const flaky = async () => {
    attempts++;
    if (attempts === 1) throw expired();
    return { agents: [AGENT] };
  };
  env.failing = false;
  const loaded = await run(client, { ...agents, queryFn: flaky });
  assert.equal(loaded.status, "success");
  assert.equal(attempts, 2);
  // A sign-out or switch stops the old account's retries.
  env.epoch = 8;
  assert.equal(agents.retry(0, expired()), false);
  h.unmount();
});

test("back in front, a pinned list that failed to load is read again; unrequested lists are left alone", async () => {
  const { h, module, client, env } = setup();
  h.mount(() => module.useQuickAccessSources({ agents: false, crews: true }));
  const crews = env.queries.at(-1);
  await client.prefetchQuery({ ...crews, retry: false });
  assert.equal(client.getQueryState(crews.queryKey)?.status, "error");
  assert.equal(env.crewReads, 1);
  env.failing = false;
  env.appState.forEach((listener) => listener("background"));
  assert.equal(env.crewReads, 1);
  env.appState.forEach((listener) => listener("active"));
  await new Promise((resolve) => setImmediate(resolve));
  same(client.getQueryData(crews.queryKey), [CREW]);
  assert.equal(env.crewReads, 2);
  assert.equal(env.agentReads, 0, "agents were not asked for");
  h.unmount();
  assert.equal(env.appState.length, 0);
  // Nothing pinned: no listener at all.
  h.mount(() => module.useQuickAccessSources({ agents: false, crews: false }));
  assert.equal(env.appState.length, 0);
  h.unmount();
});
