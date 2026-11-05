import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { deferred, hookHarness } from "../assistant/assistantHookTestHarness";

test("an edit before hydration wins and ordered persistence makes the latest layout durable", async () => {
  const stored = deferred<string | null>();
  const writes: string[] = [];
  const firstWrite = deferred<void>();
  const harness = hookHarness<any>({
    "react/jsx-runtime": {
      jsx: (type: unknown, props: unknown) => ({ type, props }),
    },
    "react-native": {},
    "@react-native-async-storage/async-storage": {
      __esModule: true,
      default: {
        getItem: () => stored.promise,
        setItem: async (_key: string, value: string) => {
          writes.push(value);
          if (writes.length === 1) await firstWrite.promise;
        },
      },
    },
  });
  const source = harness.load(
    fileURLToPath(new URL("./ChatLayoutContext.tsx", import.meta.url)),
  ) as { ChatLayoutProvider: (props: any) => any };
  harness.mount(() => source.ChatLayoutProvider({ children: null }));
  harness.value().props.value.update({ composerMaxHeight: 240 });
  harness.flush();
  stored.resolve('{"composerMaxHeight":80,"columnWidth":360}');
  await harness.settle();
  assert.equal(harness.value().props.value.layout.composerMaxHeight, 240);
  harness.value().props.value.update({ columnWidth: 900 });
  harness.value().props.value.reset();
  await harness.settle();
  assert.equal(writes.length, 1);
  firstWrite.resolve();
  await harness.settle();
  assert.deepEqual(JSON.parse(writes.at(-1)!), {
    columnWidth: 758,
    composerMaxHeight: 120,
  });
  harness.unmount();
});
