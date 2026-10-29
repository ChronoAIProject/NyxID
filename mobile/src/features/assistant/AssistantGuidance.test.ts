import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { deferred, hookHarness } from "./assistantHookTestHarness";
import { ApiError, isApiError } from "../../lib/api/ApiError";
import { submitAssistantDraft } from "./assistantComposerDraft";
import { conversationSchema } from "./assistantTypes";

const id = `nyxa-${"a".repeat(32)}`;
const failure = (code: string, status = 409) =>
  new ApiError({
    errorKey: code,
    errorCode: 400,
    statusCode: status,
    message: code,
  });
const plain = (v: any) => JSON.parse(JSON.stringify(v));
function setup() {
  let epoch = 1,
    available = true,
    ids = 0,
    refreshes = 0;
  const calls: any[] = [];
  const caps: any = { data: { steer: { max_chars: 32000, max_per_turn: 8 } } };
  let response = async (request: any): Promise<any> => ({
    client_request_id: request.clientRequestId,
    outcome: "applied",
    code: null,
  });
  const input: any = {
    scope: "thread-a",
    conversationId: id,
    turn: { turn_id: "one", steering_allowed: true, stop_requested: false },
    activeTurn: true,
    disabled: false,
    onRefresh: async () => {
      refreshes++;
    },
  };
  const h = hookHarness<any>({
    "expo-crypto": {
      randomUUID: () =>
        `aaaaaaaa-aaaa-4aaa-8aaa-${String(++ids).padStart(12, "0")}`,
    },
    "@tanstack/react-query": { useQuery: () => caps },
    "../../lib/api/ApiError": { isApiError },
    "../../lib/auth/authEpoch": { getAuthEpoch: () => epoch },
    "../groups/useGroupRuntime": {
      useGroupAvailability: () => ({
        active: available,
        available: () => available,
      }),
    },
    "../../lib/api/assistantApi": {
      assistantApi: {
        steer: (
          id: string,
          text: string,
          turnId: string,
          clientRequestId: string,
        ) => {
          const call = { id, text, turnId, clientRequestId };
          calls.push(call);
          return response(call);
        },
      },
    },
  });
  const use = h.load(
    fileURLToPath(new URL("./useAssistantGuidance.ts", import.meta.url)),
  ).useAssistantGuidance as any;
  h.mount(() => use(input));
  return {
    h,
    input,
    caps,
    calls,
    refreshes: () => refreshes,
    respond: (next: typeof response) => {
      response = next;
    },
    account: () => {
      epoch++;
      h.render();
    },
    offline: () => {
      available = false;
      h.render();
    },
  };
}

test("guidance follows real capability/turn admission and refuses text/attachment bounds before dispatch", async () => {
  for (const condition of [
    "unsupported",
    "not-ready",
    "stopping",
    "inactive",
    "disabled",
    "offline",
  ]) {
    const x = setup();
    const old = x.h.value().submit;
    if (condition === "unsupported") x.caps.data.steer = null;
    if (condition === "not-ready") x.input.turn.steering_allowed = false;
    if (condition === "stopping") x.input.turn.stop_requested = true;
    if (condition === "inactive") x.input.activeTurn = false;
    if (condition === "disabled") x.input.disabled = true;
    if (condition === "offline") x.offline();
    x.h.render();
    assert.equal(await old("Guide", 0), false, condition);
    assert.equal(x.calls.length, 0);
    x.h.unmount();
  }
  const x = setup();
  for (const [text, count] of [
    ["text", 1],
    [" ", 0],
    ["😀".repeat(32001), 0],
  ] as const)
    assert.equal(await x.h.value().submit(text, count), false);
  x.h.flush();
  assert.equal(x.h.value().notice.code, "invalid_request");
  assert.equal(x.calls.length, 0);
  assert.equal(await x.h.value().submit("😀".repeat(32000), 0), true);
  x.h.unmount();
});

test("applied guidance uses one UUID/turn, blocks concurrent presses and resyncs authoritative history", async () => {
  const x = setup();
  const wait = deferred<any>();
  x.respond(() => wait.promise);
  const first = x.h.value().submit("Focus on tests", 0);
  x.h.flush();
  assert.equal(x.h.value().submitting, true);
  assert.equal(await x.h.value().submit("Focus on tests", 0), false);
  assert.equal(x.calls.length, 1);
  assert.equal(x.calls[0].turnId, "one");
  assert.match(x.calls[0].clientRequestId, /^[\da-f-]{36}$/);
  wait.resolve({
    client_request_id: x.calls[0].clientRequestId,
    outcome: "applied",
    code: null,
  });
  assert.equal(await first, true);
  await x.h.settle();
  assert.equal(x.h.value().notice.outcome, "applied");
  assert.equal(x.refreshes(), 1);
  assert.equal(x.h.value().submitting, false);
  x.h.unmount();
});

test("definite refusals restore editable draft; starting reuses the same request while another refusal does not", async () => {
  for (const code of [
    "starting",
    "response_mismatch",
    "stop_pending",
    "invalid_request",
  ]) {
    const x = setup();
    x.respond(async () => {
      throw failure(code);
    });
    let value = "Preserve this guidance",
      revision = 0;
    const updateDraft = (text: string) => {
      value = text;
      revision++;
    };
    await submitAssistantDraft({
      draft: value,
      updateDraft,
      getRevision: () => revision,
      send: (text) => x.h.value().submit(text, 0),
    });
    await x.h.settle();
    assert.equal(value, "Preserve this guidance", code);
    const original = x.calls[0].clientRequestId;
    await x.h.value().submit(value, 0);
    assert.equal(x.calls[1].clientRequestId === original, code === "starting");
    x.h.unmount();
  }
});

test("uncertain guidance consumes text without automatic retry and reuses its UUID only for an explicit same-turn retry", async () => {
  for (const cause of [
    new Error("network lost"),
    failure("steer_unavailable", 503),
    failure("internal", 500),
  ]) {
    const x = setup();
    x.respond(async () => {
      throw cause;
    });
    let value = "Uncertain text",
      revision = 0;
    await submitAssistantDraft({
      draft: value,
      updateDraft: (text) => {
        value = text;
        revision++;
      },
      getRevision: () => revision,
      send: (text) => x.h.value().submit(text, 0),
    });
    await x.h.settle();
    assert.equal(value, "");
    assert.equal(x.calls.length, 1);
    const original = x.calls[0].clientRequestId;
    await x.h.value().submit("Uncertain text", 0);
    assert.equal(x.calls[1].clientRequestId, original);
    await x.h.value().submit("Different text", 0);
    assert.notEqual(x.calls[2].clientRequestId, original);
    x.input.turn = { ...x.input.turn, turn_id: "two" };
    x.h.render();
    await x.h.value().submit("Uncertain text", 0);
    assert.notEqual(x.calls[3].clientRequestId, original);
    x.h.unmount();
  }
});

test("ended-turn recovery is explicit, locked until idle, and retains the action on ordinary-send refusal", async () => {
  const x = setup();
  x.respond(async () => {
    throw failure("no_active_turn");
  });
  assert.equal(await x.h.value().submit("Next task", 0), true);
  await x.h.settle();
  assert.equal(x.h.value().notice.code, "no_active_turn");
  assert.equal(x.h.value().notice.text, "Next task");
  const sent: string[] = [];
  await x.h.value().sendAsMessage(async (text: string) => {
    sent.push(text);
    return true;
  });
  assert.equal(sent.length, 0);
  x.input.activeTurn = false;
  x.input.turn = null;
  x.h.render();
  await x.h.value().sendAsMessage(async () => false);
  x.h.flush();
  assert.equal(x.h.value().notice.text, "Next task");
  await x.h.value().sendAsMessage(async (text: string) => {
    sent.push(text);
    return true;
  });
  x.h.flush();
  assert.deepEqual(sent, ["Next task"]);
  assert.equal(x.h.value().notice, undefined);
  x.h.unmount();
});

test("late results and retained recovery callbacks cannot mutate another thread/account or an unmounted view", async () => {
  for (const fence of ["scope", "account", "unmount"]) {
    const x = setup();
    const wait = deferred<any>();
    x.respond(() => wait.promise);
    const pending = x.h.value().submit("Old guidance", 0);
    if (fence === "scope") {
      x.input.scope = "thread-b";
      x.h.render();
    }
    if (fence === "account") x.account();
    if (fence === "unmount") x.h.unmount();
    wait.resolve({
      client_request_id: x.calls[0].clientRequestId,
      outcome: "applied",
      code: null,
    });
    await pending;
    await x.h.settle();
    assert.equal(x.h.value().notice, undefined, fence);
    assert.equal(x.refreshes(), 0, fence);
    x.h.unmount();
  }
});

test("refresh failure cannot change an accepted guidance receipt", async () => {
  const x = setup();
  x.input.onRefresh = async () => {
    throw new Error("history offline");
  };
  x.h.render();
  assert.equal(await x.h.value().submit("Proceed", 0), true);
  await x.h.settle();
  assert.equal(x.h.value().notice.outcome, "applied");
  x.h.unmount();
});

test("capability/steer API matches web camelCase request ID and fences a different response identity", async () => {
  const calls: any[] = [];
  let mismatch = false;
  const h = hookHarness<any>({
    "./http": {
      requestJson: async (path: string, options: any) => {
        calls.push([path, options]);
        return path.endsWith("capabilities")
          ? { steer: { max_chars: 32000, max_per_turn: 8 } }
          : {
              client_request_id: mismatch ? "different" : "request",
              outcome: "applied",
              code: null,
            };
      },
    },
  });
  const api = h.load(
    fileURLToPath(new URL("../../lib/api/assistantApi.ts", import.meta.url)),
  ).assistantApi as any;
  await api.capabilities(id);
  await api.steer(id, "Guide", "turn", "request");
  assert.deepEqual(plain(calls[1]), [
    `/assistant/nyxagent/conversations/${id}/steer`,
    {
      method: "POST",
      body: { text: "Guide", turn_id: "turn", clientRequestId: "request" },
    },
  ]);
  mismatch = true;
  await assert.rejects(
    api.steer(id, "Guide", "turn", "request"),
    /could not be confirmed/,
  );
  const c = conversationSchema.parse({
    id,
    title: "Thread",
    model: "nyxagent/chat",
    created_at: "now",
    last_message_at: "now",
    message_count: 0,
    active_turn: { turn_id: "turn", started_at: "now", steering_allowed: true },
  });
  assert.equal(c.active_turn?.steering_allowed, true);
});
