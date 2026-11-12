import assert from "node:assert/strict";
import { getEventListeners } from "node:events";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { deferred, hookHarness } from "./assistantHookTestHarness";
import { HISTORY_UNAVAILABLE } from "./assistantHistoryCompat";
import {
  ASSISTANT_HISTORY_TIMEOUT_MS,
  withAssistantHistoryDeadline,
} from "./assistantHistoryRequest";
import { historySchema, type AssistantHistory } from "./assistantTypes";
import type { useAssistant } from "./useAssistant";
import type { streamAssistantTurn } from "../../lib/api/assistantApi";

const CHAT = `nyxa-${"a".repeat(32)}`;
const OTHER = `nyxa-${"b".repeat(32)}`;
const DATE = "2026-11-10T00:00:00Z";

function savedHistory(id = CHAT): AssistantHistory {
  return historySchema.parse({
    conversation: {
      id,
      title: "Saved QA conversation",
      model: "nyxagent/chat",
      created_at: DATE,
      last_message_at: DATE,
      message_count: 10,
      active_turn: null,
    },
    messages: Array.from({ length: 10 }, (_, index) => ({
      id: `message-${index}`,
      seq: index + 1,
      turn_id: `turn-${Math.floor(index / 2)}`,
      role: index % 2 ? "assistant" : "user",
      text: `QA message ${index}`,
      status: "completed",
      error_code: null,
      created_at: DATE,
      attachments:
        index === 0
          ? [0, 1].map((attachment) => ({
              id: `attachment-${attachment}`,
              content_type: "image/png",
              size: 128,
              label: "QA image",
            }))
          : [],
    })),
    before_seq: null,
  });
}

function setup() {
  let focused = true;
  let route: string | undefined = CHAT;
  let accountEpoch = 1;
  let stateChanged: ((state: string) => void) | undefined;
  let networkChanged:
    | ((state: { isConnected: boolean; isInternetReachable: boolean }) => void)
    | undefined;
  let invalidate: ((id: string) => Promise<unknown>) | undefined;
  const terminal = deferred<void>();
  const mutations: string[] = [];
  const stream: typeof streamAssistantTurn = async (
    _input,
    _onEvent,
    _controller,
    onAdmitted,
  ) => {
    mutations.push("send");
    onAdmitted?.();
    await terminal.promise;
  };
  const requests: {
    id: string;
    before?: number;
    signal: AbortSignal;
    pending: ReturnType<typeof deferred<AssistantHistory>>;
  }[] = [];
  const h = hookHarness<ReturnType<typeof useAssistant>>(
    {
      "./useAssistantLiveRefresh": {
        useAssistantLiveRefresh: (
          _active: boolean,
          _id: string | undefined,
          _paused: boolean,
          refresh: (id: string) => Promise<unknown>,
        ) => {
          invalidate = refresh;
        },
      },
      "../../lib/auth/authEpoch": { getAuthEpoch: () => accountEpoch },
      "react-native": {
        AppState: {
          currentState: "active",
          addEventListener: (_name: string, callback: typeof stateChanged) => {
            stateChanged = callback;
            return { remove() {} };
          },
        },
      },
      "@react-navigation/native": { useIsFocused: () => focused },
      "@react-native-community/netinfo": {
        addEventListener: (callback: typeof networkChanged) => {
          networkChanged = callback;
          return () => {};
        },
      },
      "@tanstack/react-query": {
        useQuery: () => ({
          data: {
            agents: [
              { id: "nyxbot", kind: "nyxbot", status: "idle", can_use: true },
            ],
          },
        }),
      },
      "../../lib/api/agentsApi": { agentsApi: {} },
      "../../lib/api/assistantApi": {
        assistantApi: {
          history: (id: string, before?: number, signal?: AbortSignal) => {
            assert.ok(signal, "history reads carry their cancellation signal");
            const pending = deferred<AssistantHistory>();
            requests.push({ id, before, signal, pending });
            return pending.promise;
          },
          list: async () => ({ conversations: [], next_cursor: null }),
          decide: async () => ({ status: "denied" }),
          decideApproval: async () => {
            mutations.push("approval");
          },
          stop: async () => {
            mutations.push("stop");
          },
        },
        streamAssistantTurn: stream,
      },
    },
    // Model the single Error realm used by the application and its API modules.
    { Error },
  );
  const hook = h.load(
    fileURLToPath(new URL("./useAssistant.ts", import.meta.url)),
  ).useAssistant as typeof useAssistant;
  h.mount(() => hook(route));
  return {
    h,
    requests,
    mutations,
    terminal,
    invalidate() {
      assert.ok(invalidate, "live history invalidation is subscribed");
      return invalidate(CHAT);
    },
    request(index: number) {
      const request = requests[index];
      assert.ok(request, `history request ${index} was dispatched`);
      return request;
    },
    foreground(active: boolean) {
      stateChanged?.(active ? "active" : "background");
      h.flush();
    },
    focus(value: boolean) {
      focused = value;
      h.render();
    },
    online(value: boolean) {
      networkChanged?.({ isConnected: value, isInternetReachable: value });
      h.flush();
    },
    route(id?: string) {
      route = id;
      h.render();
    },
    changeAccount() {
      accountEpoch += 1;
      h.render();
    },
  };
}

for (const stalledStage of ["headers", "body"] as const) {
  test(`history deadline bounds stalled ${stalledStage}, including ignored abort`, async (context) => {
    context.mock.timers.enable({ apis: ["setTimeout"] });
    const controller = new AbortController();
    const headers = deferred<{ text: () => Promise<string> }>();
    const body = deferred<string>();
    const read = withAssistantHistoryDeadline(controller, async () => {
      const response = await headers.promise;
      return response.text();
    });
    if (stalledStage === "body") headers.resolve({ text: () => body.promise });
    await new Promise<void>((resolve) => setImmediate(resolve));
    const rejected = assert.rejects(read, {
      name: "HistoryRequestTimeoutError",
      message: HISTORY_UNAVAILABLE,
    });
    context.mock.timers.tick(ASSISTANT_HISTORY_TIMEOUT_MS - 1);
    assert.equal(controller.signal.aborted, false);
    context.mock.timers.tick(1);
    await rejected;
    assert.equal(controller.signal.aborted, true);
    assert.deepEqual(getEventListeners(controller.signal, "abort"), []);
    headers.resolve({ text: () => body.promise });
    body.reject(new Error("late private response failure"));
    await new Promise<void>((resolve) => setImmediate(resolve));
  });
}

test("a saved conversation times out, exits its skeleton and can retry all messages and attachments", async (context) => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  const x = setup();
  context.after(() => x.h.unmount());
  await x.h.settle();
  assert.equal(x.requests.length, 1, "initial lifecycle reads share one GET");
  assert.equal(x.h.value().loading, true);
  context.mock.timers.tick(ASSISTANT_HISTORY_TIMEOUT_MS);
  await x.h.settle();
  assert.equal(x.request(0).signal.aborted, true);
  assert.equal(x.h.value().loading, false);
  assert.equal(x.h.value().error, HISTORY_UNAVAILABLE);
  const retry = x.h.value().recover();
  x.h.flush();
  assert.equal(x.h.value().loading, true);
  assert.equal(x.h.value().error, undefined);
  x.request(1).pending.resolve(savedHistory());
  await retry;
  await x.h.settle();
  assert.equal(x.h.value().loading, false);
  assert.equal(x.h.value().error, undefined);
  assert.equal(x.h.value().history?.messages.length, 10);
  assert.equal(
    x.h.value().history?.messages.flatMap((message) => message.attachments)
      .length,
    2,
  );
  context.mock.timers.tick(ASSISTANT_HISTORY_TIMEOUT_MS);
  assert.equal(
    x.request(1).signal.aborted,
    false,
    "successful reads clear timers",
  );
});

test("manual recovery replaces a hung initial read and owns loading completion before the old read settles", async (context) => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  const x = setup();
  context.after(() => x.h.unmount());
  const old = x.request(0);
  const retry = x.h.value().recover();
  assert.equal(old.signal.aborted, true);
  x.request(1).pending.resolve(savedHistory());
  await retry;
  await x.h.settle();
  assert.equal(x.h.value().loading, false);
  old.pending.reject(new Error("late old failure"));
  await x.h.settle();
  assert.equal(x.h.value().error, undefined);
  assert.equal(x.h.value().history?.messages.length, 10);
});

for (const lifecycle of ["foreground", "focus", "network"] as const) {
  test(`${lifecycle} restoration cancels a stalled read and reloads authoritative history`, async (context) => {
    context.mock.timers.enable({ apis: ["setTimeout"] });
    const x = setup();
    context.after(() => x.h.unmount());
    const old = x.request(0);
    const change =
      lifecycle === "foreground"
        ? x.foreground
        : lifecycle === "focus"
          ? x.focus
          : x.online;
    change(false);
    await x.h.settle();
    assert.equal(old.signal.aborted, true);
    assert.equal(x.requests.length, 1);
    change(true);
    await x.h.settle();
    assert.equal(x.requests.length, 2);
    x.request(1).pending.resolve(savedHistory());
    await x.h.settle();
    assert.equal(x.h.value().loading, false);
    assert.equal(x.h.value().error, undefined);
    old.pending.reject(new Error("late background failure"));
    await x.h.settle();
    assert.equal(x.h.value().error, undefined);
  });
}

test("a hung or failed chat never prevents switching chats or returning home", async (context) => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  const x = setup();
  context.after(() => x.h.unmount());
  const old = x.request(0);
  x.route(OTHER);
  await x.h.settle();
  assert.equal(old.signal.aborted, true);
  assert.equal(x.h.value().conversationId, OTHER);
  assert.equal(x.h.value().loading, true);
  old.pending.resolve(savedHistory());
  await x.h.settle();
  assert.equal(x.h.value().history, undefined);
  assert.equal(
    x.h.value().loading,
    true,
    "old completion cannot release new loading",
  );
  context.mock.timers.tick(ASSISTANT_HISTORY_TIMEOUT_MS);
  await x.h.settle();
  assert.equal(x.h.value().loading, false);
  assert.equal(x.h.value().error, HISTORY_UNAVAILABLE);
  x.route();
  await x.h.settle();
  assert.equal(x.h.value().conversationId, undefined);
  assert.equal(x.h.value().loading, false);
  assert.equal(x.h.value().error, undefined);
});

test("account change or unmount cancels history without admitting late results or errors", async (context) => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  for (const boundary of ["account", "unmount"] as const) {
    const x = setup();
    const request = x.request(0);
    if (boundary === "account") x.changeAccount();
    else x.h.unmount();
    await x.h.settle();
    assert.equal(request.signal.aborted, true);
    request.pending.resolve(savedHistory());
    await x.h.settle();
    assert.equal(x.h.value().history, undefined);
    assert.equal(x.h.value().error, undefined);
    if (boundary === "account") {
      await x.h.value().recover();
      assert.equal(
        x.requests.length,
        1,
        "old-account screen cannot issue new reads",
      );
      x.h.unmount();
    }
  }
});

test("paging history keeps existing rows and releases its own loading on timeout", async (context) => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  const x = setup();
  context.after(() => x.h.unmount());
  const page = savedHistory();
  page.before_seq = 5;
  page.messages = page.messages.filter((message) => message.seq >= 5);
  x.request(0).pending.resolve(page);
  await x.h.settle();
  const earlier = x.h.value().loadOlder();
  x.h.flush();
  assert.equal(x.h.value().loadingOlder, true);
  assert.equal(x.request(1).before, 5);
  context.mock.timers.tick(ASSISTANT_HISTORY_TIMEOUT_MS);
  await earlier;
  await x.h.settle();
  assert.equal(x.h.value().loadingOlder, false);
  assert.equal(x.h.value().history?.messages.length, 6);
  assert.equal(x.h.value().error, HISTORY_UNAVAILABLE);
  const retry = x.h.value().loadOlder();
  const olderPage = savedHistory();
  olderPage.messages = olderPage.messages.filter((message) => message.seq < 5);
  x.request(2).pending.resolve(olderPage);
  await retry;
  await x.h.settle();
  assert.equal(x.h.value().history?.messages.length, 10);
  assert.equal(x.h.value().history?.before_seq, null);
});

test("a decision invalidates a pending snapshot instead of sharing the pre-decision read", async (context) => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  const x = setup();
  context.after(() => x.h.unmount());
  x.request(0).pending.resolve(savedHistory());
  await x.h.settle();
  const refresh = x.h.value().recover();
  const stale = x.request(1);
  const decision = x.h.value().decide("confirmation", "deny");
  await x.h.settle();
  assert.equal(stale.signal.aborted, true);
  const latest = savedHistory();
  latest.conversation.title = "After decision";
  x.request(2).pending.resolve(latest);
  await decision;
  await refresh;
  await x.h.settle();
  stale.pending.resolve(savedHistory());
  await x.h.settle();
  assert.equal(x.h.value().history?.conversation.title, "After decision");
  assert.equal(x.h.value().deciding, undefined);
  assert.equal(x.h.value().error, undefined);
});

for (const boundary of ["stop", "approval", "send", "live"] as const) {
  test(`${boundary} reconciliation replaces a pre-write snapshot and ignores its late result`, async (context) => {
    context.mock.timers.enable({ apis: ["setTimeout"] });
    const x = setup();
    context.after(() => x.h.unmount());
    const before = savedHistory();
    if (boundary === "stop") {
      before.conversation.active_turn = {
        turn_id: "running",
        started_at: DATE,
      };
    }
    if (boundary === "approval") {
      before.approvals = [
        {
          id: "approval",
          service_name: "QA service",
          summary: "QA confirmation",
          expires_at: new Date(Date.now() + 60_000).toISOString(),
        },
      ];
    }
    x.request(0).pending.resolve(before);
    await x.h.settle();
    const sent =
      boundary === "send" ? x.h.value().send("QA message") : undefined;
    x.h.flush();
    const refresh = x.h.value().recover();
    const stale = x.request(1);
    const operation =
      boundary === "stop"
        ? x.h.value().stop()
        : boundary === "approval"
          ? x.h.value().decideApproval("approval", true)
          : boundary === "live"
            ? x.invalidate()
            : sent;
    if (boundary === "send") x.terminal.resolve();
    await x.h.settle();
    assert.equal(
      stale.signal.aborted,
      true,
      "authoritative reconciliation cannot reuse the pre-write GET",
    );
    const latest = savedHistory();
    latest.conversation.title = "Saved after change";
    x.request(2).pending.resolve(latest);
    await operation;
    await refresh;
    await x.h.settle();
    assert.equal(x.h.value().history?.conversation.title, "Saved after change");
    assert.equal(x.h.value().history?.conversation.active_turn, null);
    assert.equal(x.h.value().history?.approvals.length, 0);
    assert.equal(x.h.value().sending, false);
    assert.equal(x.h.value().mutating, false);
    assert.equal(x.h.value().deciding, undefined);
    stale.pending.resolve(before);
    await x.h.settle();
    assert.equal(x.h.value().history?.conversation.title, "Saved after change");
    assert.equal(x.h.value().error, undefined);
    assert.deepEqual(x.mutations, boundary === "live" ? [] : [boundary]);
  });
}

test("a live replacement of initial history exposes its own timeout and retry action", async (context) => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  const x = setup();
  context.after(() => x.h.unmount());
  const initial = x.request(0);
  const replacement = assert.rejects(x.invalidate(), {
    name: "HistoryRequestTimeoutError",
    message: HISTORY_UNAVAILABLE,
  });
  await x.h.settle();
  assert.equal(initial.signal.aborted, true);
  assert.equal(x.h.value().loading, true);
  context.mock.timers.tick(ASSISTANT_HISTORY_TIMEOUT_MS);
  await replacement;
  await x.h.settle();
  assert.equal(x.h.value().loading, false);
  assert.equal(x.h.value().history, undefined);
  assert.equal(x.h.value().error, HISTORY_UNAVAILABLE);
  const retry = x.h.value().recover();
  x.request(2).pending.resolve(savedHistory());
  await retry;
  await x.h.settle();
  assert.equal(x.h.value().history?.messages.length, 10);
  assert.equal(x.h.value().loading, false);
  assert.equal(x.h.value().error, undefined);
});
