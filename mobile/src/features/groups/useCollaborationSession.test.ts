import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { deferred, hookHarness } from "../assistant/assistantHookTestHarness";

function setup() {
  let epoch = 1,
    active = true,
    identityError = false;
  let features = ["assistant:shared-agents"];
  let capability = true;
  let pending:
    | ReturnType<typeof deferred<{ id: string; enabled_features: string[] }>>
    | undefined;
  const calls: string[] = [];
  const harness = hookHarness<any>({
    "@tanstack/react-query": {
      useQueryClient: () => ({
        invalidateQueries: async () => {},
        cancelQueries: async () => {},
      }),
      useQuery: (options: any) =>
        options.queryKey.at(-1) === "identity"
          ? {
              data: {
                id: "actor",
                enabled_features: ["assistant:shared-agents"],
              },
              isError: identityError,
              isFetching: false,
              dataUpdatedAt: Number.MAX_SAFE_INTEGER,
            }
          : {
              data: capability,
              isError: false,
              isFetching: false,
              dataUpdatedAt: Number.MAX_SAFE_INTEGER,
            },
    },
    "../../lib/api/groupsApi": {
      groupsApi: {
        identity: async () => {
          calls.push("identity");
          return pending
            ? pending.promise
            : { id: "actor", enabled_features: features };
        },
      },
    },
    "../../lib/api/collaborationApi": {
      collaborationApi: {
        available: async () => {
          calls.push("capability");
          return capability;
        },
      },
    },
    "../../lib/auth/authEpoch": { getAuthEpoch: () => epoch },
    "./useGroupRuntime": {
      groupKeys: (value: number) => ["groups", value],
      useGroupAvailability: () => ({ epoch, active, available: () => active }),
    },
  });
  const { useCollaborationSession } = harness.load(
    fileURLToPath(new URL("./useCollaborationSession.ts", import.meta.url)),
  ) as any;
  harness.mount(() => useCollaborationSession("assistant:shared-agents", true));
  return {
    harness,
    calls,
    get session() {
      return harness.flush();
    },
    render: () => harness.render(),
    setActive: (value: boolean) => {
      active = value;
      harness.render();
    },
    setEpoch: (value: number) => {
      epoch = value;
      harness.render();
    },
    setFeatures: (value: string[]) => {
      features = value;
    },
    setCapability: (value: boolean) => {
      capability = value;
    },
    failIdentity: () => {
      identityError = true;
      harness.render();
    },
    delayIdentity: () => {
      pending = deferred();
      return pending;
    },
  };
}

test("new collaboration actions re-read feature and fleet capability before writing", async () => {
  const env = setup();
  let writes = 0;
  await env.session.run(async () => {
    writes++;
  });
  assert.equal(writes, 1);
  assert.deepEqual(env.calls, ["identity", "capability"]);
  env.setFeatures([]);
  await env.session.run(async () => {
    writes++;
  });
  assert.equal(writes, 1);
  assert.match(env.session.error, /no longer enabled/);
  env.setFeatures(["assistant:shared-agents"]);
  env.setCapability(false);
  await env.session.run(async () => {
    writes++;
  });
  assert.equal(writes, 1);
  assert.match(env.session.error, /rollout/);
});

test("failed identity refresh never treats retained feature data as current authority", () => {
  const env = setup();
  assert.equal(env.session.enabled, true);
  env.failIdentity();
  assert.equal(env.session.enabled, false);
  assert.equal(env.session.readable, false);
});

test("account switch or background during asynchronous admission prevents any write or success callback", async () => {
  for (const boundary of ["account", "background"] as const) {
    const env = setup();
    const pending = env.delayIdentity();
    let writes = 0,
      success = 0;
    const result = env.session.run(
      async () => {
        writes++;
      },
      () => {
        success++;
      },
    );
    if (boundary === "account") env.setEpoch(2);
    else env.setActive(false);
    pending.resolve({
      id: "actor",
      enabled_features: ["assistant:shared-agents"],
    });
    await result;
    assert.equal(writes, 0);
    assert.equal(success, 0);
    if (boundary === "background") assert.equal(env.session.readable, false);
  }
});

test("a queued transport guard remains invalid after background and foreground, even within one millisecond", () => {
  const env = setup();
  const guard = env.session.guard;
  guard();
  env.setActive(false);
  env.setActive(true);
  assert.throws(guard, /inactive|account changed/);
  env.session.guard();
});
