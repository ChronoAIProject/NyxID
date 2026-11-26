import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath, URL } from "node:url";
import {
  deferred,
  hookHarness,
} from "../../features/assistant/assistantHookTestHarness";

type Hidden = readonly [boolean, (hidden: boolean) => void];

function setup(saved: string | null | Error) {
  const stored = new Map<string, string>();
  const reads: string[] = [];
  const read = deferred<string | null>();
  const storage = {
    getItem: (key: string) => {
      reads.push(key);
      return read.promise;
    },
    setItem: async (key: string, value: string) => {
      stored.set(key, value);
    },
  };
  const h = hookHarness<Hidden>({
    "@react-native-async-storage/async-storage": {
      __esModule: true,
      default: storage,
    },
  });
  // The shared stored-flag hook loads through the harness's React too.
  h.mock(
    "./workspaceDevicePreference",
    h.load(fileURLToPath(new URL("./workspaceDevicePreference.ts", import.meta.url))),
  );
  const module = h.load(
    fileURLToPath(new URL("./workspaceQuickRailPreference.ts", import.meta.url)),
  ) as any;
  h.mount(() => module.useQuickRailHidden());
  const resolve = async () => {
    if (saved instanceof Error) read.reject(saved);
    else read.resolve(saved);
    return h.settle();
  };
  return { h, module, stored, reads, resolve };
}

test("full-screen mode defaults to the rail shown and restores a saved choice per device", async () => {
  for (const [saved, hidden] of [
    [null, false],
    ["0", false],
    ["1", true],
    [new Error("storage unavailable"), false],
  ] as const) {
    const x = setup(saved);
    assert.equal(x.h.value()[0], false, "shown until the saved value loads");
    await x.resolve();
    assert.equal(x.h.value()[0], hidden, String(saved));
    assert.deepEqual(x.reads, [x.module.QUICK_RAIL_HIDDEN_KEY]);
    x.h.unmount();
  }
});

test("Full screen and Show quick actions persist, and a choice before the load wins", async () => {
  const x = setup("0");
  x.h.value()[1](true);
  x.h.flush();
  assert.equal(x.h.value()[0], true);
  await x.resolve();
  assert.equal(x.h.value()[0], true, "the late saved value cannot undo a tap");
  assert.equal(x.stored.get(x.module.QUICK_RAIL_HIDDEN_KEY), "1");
  x.h.value()[1](false);
  x.h.flush();
  await x.h.settle();
  assert.equal(x.h.value()[0], false);
  assert.equal(x.stored.get(x.module.QUICK_RAIL_HIDDEN_KEY), "0");
  x.h.unmount();
});
