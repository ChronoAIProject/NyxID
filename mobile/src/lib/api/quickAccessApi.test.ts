import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath, URL } from "node:url";
import { hookHarness } from "../../features/assistant/assistantHookTestHarness";
import {
  QUICK_ACCESS_DEFAULT_STATE,
  QUICK_ACCESS_DESTINATIONS,
  quickAccessResponseSchema,
  sameQuickAccessItem,
} from "./quickAccessSchema";

const AGENT = "6f9619ff-8b86-d011-b42d-00c04fc964ff";
const CREW = "nyxg-0123456789abcdef0123456789abcdef";
// The harness evaluates the module in its own realm: compare objects as JSON.
const same = (actual: unknown, expected: unknown) =>
  assert.equal(JSON.stringify(actual), JSON.stringify(expected));

function api(answer: unknown) {
  const calls: { path: string; options: any }[] = [];
  const h = hookHarness<unknown>({
    "./http": {
      requestJson: async (path: string, options: unknown = {}) => {
        calls.push({ path, options });
        return answer;
      },
    },
  });
  const { quickAccessApi } = h.load(
    fileURLToPath(new URL("./quickAccessApi.ts", import.meta.url)),
  ) as { quickAccessApi: any };
  return { quickAccessApi, calls };
}

test("the default list is the rail's original five, and the closed destination list matches the server's", () => {
  same(QUICK_ACCESS_DEFAULT_STATE, {
    items: ["new_chat", "notifications", "scan_qr", "recent_chats", "settings"].map(
      (id) => ({ kind: "destination", id }),
    ),
    customized: false,
  });
  // backend/src/models/user.rs QuickAccessDestination, in the same order.
  assert.deepEqual(
    [...QUICK_ACCESS_DESTINATIONS],
    [
      "new_chat",
      "notifications",
      "scan_qr",
      "terminal_login_code",
      "recent_chats",
      "automations",
      "machines",
      "plugins",
      "activity",
      "create_agent",
      "settings",
      "agents",
      "crews",
    ],
  );
});

test("responses keep known entries in order and skip what a newer server may add", () => {
  const parsed = quickAccessResponseSchema.parse({
    customized: true,
    items: [
      { kind: "destination", id: "plugins" },
      { kind: "destination", id: "billing" },
      { kind: "workflow", id: "x" },
      { kind: "agent", id: AGENT },
      { kind: "agent", id: "not-a-uuid" },
      { kind: "crew", id: CREW },
      "garbage",
    ],
  });
  assert.deepEqual(parsed, {
    customized: true,
    hasUnsupportedEntries: true,
    items: [
      { kind: "destination", id: "plugins" },
      { kind: "agent", id: AGENT },
      { kind: "crew", id: CREW },
    ],
  });
  assert.equal(quickAccessResponseSchema.safeParse({ items: [] }).success, false);
  assert.ok(sameQuickAccessItem({ kind: "agent", id: AGENT }, { kind: "agent", id: AGENT.toUpperCase() }));
  assert.ok(!sameQuickAccessItem({ kind: "agent", id: AGENT }, { kind: "crew", id: CREW }));
});

test("get, save and reset use the account's quick-access route; saves are checked before sending", async () => {
  const answer = { items: [{ kind: "agent", id: AGENT }], customized: true };
  const parsed = { customized: true, items: [{ kind: "agent", id: AGENT }] };
  const { quickAccessApi, calls } = api(answer);
  same(await quickAccessApi.get(), parsed);
  same(await quickAccessApi.save([{ kind: "agent", id: AGENT }]), parsed);
  same(await quickAccessApi.reset(), parsed);
  same(
    calls.map(({ path, options }) => [path, options.method ?? "GET", options.body]),
    [
      ["/users/me/quick-access", "GET", undefined],
      ["/users/me/quick-access", "PUT", { items: [{ kind: "agent", id: AGENT }] }],
      ["/users/me/quick-access", "DELETE", undefined],
    ],
  );
  // More than 12, or an entry the server would reject, never leaves the app.
  const thirteen = Array.from({ length: 13 }, (_, index) => ({
    kind: "destination",
    id: ["new_chat", "notifications", "scan_qr"][index % 3],
  }));
  await assert.rejects(quickAccessApi.save(thirteen));
  await assert.rejects(quickAccessApi.save([{ kind: "destination", id: "billing" }]));
  assert.equal(calls.length, 3);
  // An unusable answer is a translated error, not the payload.
  const broken = api({ unexpected: true });
  await assert.rejects(broken.quickAccessApi.get(), /unexpected response/);
});


test('M2622-08 server partial flag survives even when the old backend already filtered unknown items',()=>{
  const state=quickAccessResponseSchema.parse({items:[{kind:'destination',id:'plugins'}],customized:true,has_unsupported_entries:true});
  assert.equal(state.hasUnsupportedEntries,true);
  assert.equal(quickAccessResponseSchema.parse({items:[],customized:true}).hasUnsupportedEntries,undefined);
});
