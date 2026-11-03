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

test("revocation can explicitly drop admission gates while omitted feature keeps the default", async () => {
  const env = setup();
  env.setFeatures([]);
  env.setCapability(false);
  let writes = 0;
  await env.session.run(
    async () => {
      writes++;
    },
    undefined,
    { feature: null, capability: false },
  );
  assert.equal(writes, 1);
  assert.deepEqual(env.calls, ["identity"]);
  for (const authority of [
    undefined,
    { feature: undefined, capability: false },
  ]) {
    await env.session.run(
      async () => {
        writes++;
      },
      undefined,
      authority,
    );
    assert.equal(writes, 1);
    assert.match(env.session.error, /no longer enabled/);
  }
  // Dropping the feature gate does not implicitly drop a required capability.
  await env.session.run(
    async () => {
      writes++;
    },
    undefined,
    { feature: null },
  );
  assert.equal(writes, 1);
  assert.match(env.session.error, /rollout/);
});

test("revocation override never bypasses account or foreground authority", async () => {
  for (const boundary of ["account", "background"] as const) {
    const env = setup();
    const pending = env.delayIdentity();
    let writes = 0;
    const result = env.session.run(
      async () => {
        writes++;
      },
      undefined,
      { feature: null, capability: false },
    );
    if (boundary === "account") env.setEpoch(2);
    else env.setActive(false);
    pending.resolve({ id: "actor", enabled_features: [] });
    await result;
    assert.equal(writes, 0);
  }
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

test("action errors clear across foreground and account contexts without reviving on return", async () => {
  const env = setup();
  const fail = () =>
    env.session.run(async () => {
      throw new Error("Invitation is invalid");
    });
  await fail();
  assert.ok(env.session.error);
  env.setActive(false);
  assert.equal(env.session.error, undefined);
  env.setActive(true);
  assert.equal(env.session.error, undefined);
  await fail();
  assert.ok(env.session.error);
  env.setEpoch(2);
  assert.equal(env.session.error, undefined);
  env.setEpoch(1);
  assert.equal(env.session.error, undefined);
});

test("clearError only dismisses feedback and does not release an in-flight lock or weaken admission", async () => {
  const env = setup();
  env.setFeatures([]);
  let writes = 0;
  await env.session.run(async () => {
    writes++;
  });
  assert.match(env.session.error, /no longer enabled/);
  env.session.clearError();
  assert.equal(env.session.error, undefined);
  await env.session.run(async () => {
    writes++;
  });
  assert.equal(writes, 0);
  assert.match(env.session.error, /no longer enabled/);
  const pending = env.delayIdentity();
  const operation = env.session.run(async () => {
    writes++;
  });
  assert.equal(env.session.busy, true);
  env.session.clearError();
  assert.equal(env.session.busy, true);
  await env.session.run(async () => {
    writes++;
  });
  assert.equal(writes, 0);
  // The pending operation also remains fenced if this context is abandoned.
  env.setActive(false);
  env.setActive(true);
  pending.resolve({
    id: "actor",
    enabled_features: ["assistant:shared-agents"],
  });
  await operation;
  assert.equal(writes, 0);
  assert.equal(env.session.error, undefined);
  assert.equal(env.session.busy, false);
});
