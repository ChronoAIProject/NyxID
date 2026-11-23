import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { QueryClient, QueryObserver, onlineManager } from "@tanstack/query-core";
import { buildApiError } from "../../lib/api/responseError";
import { hookHarness } from "./assistantHookTestHarness";

test("shared permission queries recover from 429 after reconnect and respect the active view", async (t) => {
  t.mock.timers.enable({ apis: ["Date", "setTimeout"], now: 1_000 });
  const client = new QueryClient();
  const wasOnline = onlineManager.isOnline();
  onlineManager.setOnline(false);
  client.mount();
  const calls: number[] = [];
  const errors = [
    buildApiError(null, 429, { get: (name) => name === "Retry-After" ? "8" : null }),
    buildApiError(null, 429),
  ];
  const settings = async () => {
    calls.push(Date.now());
    const failure = errors.shift();
    if (failure) throw failure;
    return { access_mode: "nyxmode" };
  };
  const readers = Array.from({ length: 2 }, () => {
    let active = true;
    let options: any;
    let current: any = {};
    const h = hookHarness({
      "@tanstack/react-query": {
        onlineManager,
        useQueryClient: () => client,
        useQuery: (next: any) => {
          options = next;
          return current;
        },
      },
      "../../lib/api/assistantApi": { assistantApi: { settings } },
    });
    const { useComposerSettingsQuery } = h.load(
      fileURLToPath(new URL("./useComposerSettingsQuery.ts", import.meta.url)),
    ) as typeof import("./useComposerSettingsQuery");
    const available = () => active && onlineManager.isOnline();
    h.mount(() => useComposerSettingsQuery(1, active, available));
    const query = new QueryObserver(client, options);
    const update = () => {
      current = query.getCurrentResult();
      h.render();
      query.setOptions(options);
    };
    const unsubscribe = query.subscribe(update);
    return {
      active: (value: boolean) => { active = value; update(); },
      close: () => { h.unmount(); unsubscribe(); },
    };
  });
  t.after(() => {
    for (const reader of readers) reader.close();
    client.unmount();
    client.clear();
    onlineManager.setOnline(wasOnline);
  });
  const flush = async () => {
    for (let i = 0; i < 12; i++) await Promise.resolve();
  };
  assert.deepEqual(calls, []);
  onlineManager.setOnline(true);
  await flush();
  assert.deepEqual(calls, [1_000]);
  t.mock.timers.tick(7_999);
  await flush();
  assert.equal(calls.length, 1);
  t.mock.timers.tick(1);
  await flush();
  assert.deepEqual(calls, [1_000, 9_000]);

  readers.forEach((reader) => reader.active(false));
  onlineManager.setOnline(false);
  t.mock.timers.tick(20_000);
  await flush();
  assert.equal(calls.length, 2);
  onlineManager.setOnline(true);
  readers.forEach((reader) => reader.active(true));
  t.mock.timers.tick(1);
  await flush();
  assert.equal(calls.length, 3);
  assert.equal(client.getQueryState(["assistant", "composer-settings", 1])?.status, "success");
  t.mock.timers.tick(60_000);
  await flush();
  assert.equal(calls.length, 3);

  errors.push(buildApiError(null, 403));
  await client.refetchQueries({ queryKey: ["assistant", "composer-settings", 1] });
  assert.equal(calls.length, 4);
  readers.forEach((reader) => reader.active(false));
  readers.forEach((reader) => reader.active(true));
  t.mock.timers.tick(60_000);
  await flush();
  assert.equal(calls.length, 4);
  assert.equal(client.getQueryState(["assistant", "composer-settings", 1])?.status, "error");
});
