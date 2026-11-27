import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { deferred, hookHarness } from "./assistantHookTestHarness";
import { historySchema, type AssistantHistory } from "./assistantTypes";
import { AssistantTurnPerformance, CHAT_COMMIT_GRACE_MS, type ChatTurnTimings } from "./assistantTurnPerformance";
import type { useAssistant } from "./useAssistant";
import { buildApiError } from "../../lib/api/responseError";
import type { streamAssistantTurn } from "../../lib/api/assistantApi";

const A = `nyxa-${"a".repeat(32)}`;
const B = `nyxa-${"b".repeat(32)}`;
function history(id: string): AssistantHistory {
  return historySchema.parse({
    conversation: {
      id,
      title: id,
      model: "nyxagent/chat",
      created_at: "2026-10-15T00:00:00Z",
      last_message_at: "2026-10-15T00:00:00Z",
      message_count: 0,
      active_turn: null,
    },
    messages: [],
    acknowledgements: [],
    approvals: [],
    waiting: [],
    before_seq: null,
  });
}

async function harness(overrideStream?: typeof streamAssistantTurn) {
  const agent = {
    id: "nyxbot",
    kind: "nyxbot",
    status: "idle",
    can_use: true,
    model: "nyxagent/chat",
  };
  const agentList = { agents: [agent] };
  let hold: "history" | "index" | undefined;
  let held = false;
  let historyError: Error | undefined;
  const refresh = deferred<AssistantHistory>();
  const indexRefresh = deferred<{ conversations: []; next_cursor: null }>();
  const streams: {
    done: ReturnType<typeof deferred<void>>;
    controller: AbortController;
  }[] = [];
  const stream: typeof streamAssistantTurn = async (
    _input,
    onEvent,
    controller,
    onAccepted,
  ) => {
    const id = streams.length ? B : A;
    const done = deferred<void>();
    streams.push({ done, controller });
    onAccepted?.();
    onEvent({
      event: "turn.status",
      cursor: 1,
      conversation_id: id,
      turn_id: `turn-${id}`,
      status: "running",
    });
    await done.promise;
  };
  const h = hookHarness<ReturnType<typeof useAssistant>>({
    "./useAssistantLiveRefresh": { useAssistantLiveRefresh: () => {} },
    "react-native": {
      AppState: {
        currentState: "active",
        addEventListener: () => ({ remove() {} }),
      },
    },
    "@react-navigation/native": { useIsFocused: () => true },
    "@react-native-community/netinfo": { addEventListener: () => () => {} },
    "@tanstack/react-query": {
      useQuery: () => ({
        data: agentList,
        isPending: false,
        isError: false,
        refetch: async () => {},
      }),
    },
    "../../lib/api/agentsApi": { agentsApi: { list: async () => agentList } },
    "../../lib/api/assistantApi": {
      streamAssistantTurn: overrideStream ?? stream,
      assistantApi: {
        history: async (id: string) => {
          if (historyError) throw historyError;
          if (hold === "history" && id === A) {
            hold = undefined;
            held = true;
            return refresh.promise;
          }
          return history(id);
        },
        list: async () => {
          if (hold === "index") {
            hold = undefined;
            held = true;
            return indexRefresh.promise;
          }
          return { conversations: [], next_cursor: null };
        },
        models: async () => [],
      },
    },
  });
  const hook = h.load(
    fileURLToPath(new URL("./useAssistant.ts", import.meta.url)),
  ).useAssistant as typeof useAssistant;
  h.mount(() => hook(A));
  await h.settle();
  return {
    h,
    streams,
    setHistoryError: (error?: Error) => { historyError = error; },
    hold: (phase: "history" | "index") => {
      hold = phase;
    },
    held: () => held,
    release: () => {
      refresh.resolve(history(A));
      indexRefresh.resolve({ conversations: [], next_cursor: null });
    },
  };
}

test("switching chats during either final refresh always releases the finished send's UI", async () => {
  for (const phase of ["history", "index"] as const) {
    const x = await harness();
    const send = x.h.value().send("first message");
    x.h.flush();
    assert.equal(x.h.value().sending, true);
    x.hold(phase);
    x.streams[0]!.done.resolve();
    await x.h.settle();
    assert.equal(x.held(), true, phase);
    x.h.value().openConversation();
    x.h.flush();
    assert.equal(x.h.value().conversationId, undefined);
    x.release();
    await send;
    await x.h.settle();
    assert.equal(x.h.value().sending, false, phase);
    assert.equal(x.h.value().live, undefined);
    assert.equal(x.h.value().inputText, undefined);
    x.h.unmount();
  }
});

test("old final refresh cannot clear the new send's lock, live reply or optimistic input", async () => {
  const x = await harness();
  const first = x.h.value().send("first message");
  x.h.flush();
  x.hold("history");
  x.streams[0]!.done.resolve();
  await x.h.settle();
  assert.equal(x.held(), true);
  x.h.value().openConversation();
  x.h.flush();
  const second = x.h.value().send("second message");
  await x.h.settle();
  assert.equal(x.h.value().sending, true);
  assert.equal(x.h.value().live?.conversationId, B);
  assert.equal(x.h.value().inputText, "second message");
  x.release();
  await first;
  await x.h.settle();
  assert.equal(x.h.value().sending, true);
  assert.equal(x.h.value().live?.conversationId, B);
  assert.equal(x.h.value().inputText, "second message");
  assert.equal(x.streams[1]!.controller.signal.aborted, false);
  await assert.rejects(x.h.value().send("third message"), /already being sent/);
  assert.equal(x.streams.length, 2);
  x.streams[1]!.done.resolve();
  await second;
  await x.h.settle();
  assert.equal(x.h.value().sending, false);
  assert.equal(x.h.value().live, undefined);
  x.h.unmount();
});

test("unmount during final refresh cannot publish terminal UI into the released hook", async () => {
  const x = await harness();
  const send = x.h.value().send("message");
  x.h.flush();
  x.hold("history");
  x.streams[0]!.done.resolve();
  await x.h.settle();
  x.h.unmount();
  x.release();
  await send;
  assert.equal(x.streams.length, 1);
});

test("server recovery is checked before retry; concurrent taps cannot replay it", async () => {
  const request = deferred<{
    status: "available";
    reason: string;
    text: string;
    attachments: string[];
  }>();
  let sends = 0;
  const page = history(A);
  page.messages = [
    {
      id: "reply",
      seq: 2,
      turn_id: "failed-turn",
      role: "assistant",
      text: "",
      status: "failed",
      error_code: "model_interrupted",
      created_at: "2026-10-28T00:00:00Z",
      activities: [],
      attachments: [],
    },
  ];
  const h = hookHarness<ReturnType<typeof useAssistant>>({
    "./useAssistantLiveRefresh": { useAssistantLiveRefresh: () => {} },
    "react-native": {
      AppState: {
        currentState: "active",
        addEventListener: () => ({ remove() {} }),
      },
    },
    "@react-navigation/native": { useIsFocused: () => true },
    "@react-native-community/netinfo": { addEventListener: () => () => {} },
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
        history: async () => page,
        list: async () => ({ conversations: [], next_cursor: null }),
        models: async () => [],
        recovery: async () => request.promise,
      },
      streamAssistantTurn: async (input: any) => {
        sends++;
        assert.equal(input.text, "Verified original");
        assert.equal(input.retryOf, "failed-turn");
      },
    },
  });
  const hook = h.load(
    fileURLToPath(new URL("./useAssistant.ts", import.meta.url)),
  ).useAssistant as typeof useAssistant;
  h.mount(() => hook(A));
  await h.settle();
  const one = h.value().retry("reply");
  const two = h.value().retry("reply");
  assert.equal(sends, 0);
  request.resolve({
    status: "available",
    reason: "available",
    text: "Verified original",
    attachments: [],
  });
  await Promise.allSettled([one, two]);
  await h.settle();
  assert.equal(sends, 1);
  h.value().openConversation();
  h.flush();
  await h
    .value()
    .retry("reply")
    .catch(() => undefined);
  assert.equal(sends, 1);
  h.unmount();
});

test("an allowed card resumes the chat as a continuation bound to that card", async () => {
  const card = "d9428888-122b-41e1-b85c-61cd3cbb3210";
  const sent: any[] = [];
  const page = history(A);
  const allowed = {
    id: card,
    conversation_id: A,
    kind: "action",
    status: "allowed",
    summary: "Create a scheduled automation",
    decider: "user",
    service_slug: null,
    service_name: null,
    tool_name: "nyxid__create_schedule",
    created_at: "2026-11-03T00:00:00Z",
    expires_at: "2099-01-01T00:00:00Z",
  };
  const h = hookHarness<ReturnType<typeof useAssistant>>({
    "./useAssistantLiveRefresh": { useAssistantLiveRefresh: () => {} },
    "react-native": {
      AppState: {
        currentState: "active",
        addEventListener: () => ({ remove() {} }),
      },
    },
    "@react-navigation/native": { useIsFocused: () => true },
    "@react-native-community/netinfo": { addEventListener: () => () => {} },
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
        history: async () => page,
        list: async () => ({ conversations: [], next_cursor: null }),
        models: async () => [],
        decide: async () => allowed,
      },
      streamAssistantTurn: async (input: any) => {
        sent.push(input);
      },
    },
  });
  const hook = h.load(
    fileURLToPath(new URL("./useAssistant.ts", import.meta.url)),
  ).useAssistant as typeof useAssistant;
  h.mount(() => hook(A));
  await h.settle();
  await h.value().decide(card, "allow");
  await h.settle();
  await h.settle();
  assert.equal(sent.length, 1);
  assert.equal(sent[0].conversationId, A);
  assert.equal(sent[0].acknowledgementId, card);
  assert.match(sent[0].text, /^Confirmed: Create a scheduled automation/);
  assert.match(sent[0].text, /continue the original task and tell the user when it is finished or when you need their input\.$/);
  h.unmount();
});

test("returning home from another page while a chat was selected does not leave the home loading", async () => {
  let focused = true;
  let route: string | undefined = A;
  const h = hookHarness<ReturnType<typeof useAssistant>>({
    "./useAssistantLiveRefresh": { useAssistantLiveRefresh: () => {} },
    "react-native": {
      AppState: {
        currentState: "active",
        addEventListener: () => ({ remove() {} }),
      },
    },
    "@react-navigation/native": { useIsFocused: () => focused },
    "@react-native-community/netinfo": { addEventListener: () => () => {} },
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
        history: async (id: string) => history(id),
        list: async () => ({ conversations: [], next_cursor: null }),
        models: async () => [],
      },
      streamAssistantTurn: async () => {},
    },
  });
  const hook = h.load(
    fileURLToPath(new URL("./useAssistant.ts", import.meta.url)),
  ).useAssistant as typeof useAssistant;
  h.mount(() => hook(route));
  await h.settle();
  assert.equal(h.value().loading, false);
  // Another workspace page covers the chat.
  focused = false;
  h.render();
  await h.settle();
  // The drawer's Chat button refocuses the same screen as NyxBot home.
  focused = true;
  route = undefined;
  h.render();
  await h.settle();
  await h.settle();
  assert.equal(h.value().conversationId, undefined);
  assert.equal(h.value().loading, false);
  h.unmount();
});

test("2345 malformed history never leaks validation JSON or creates a recovery lock", async () => {
  const x = await harness();
  x.setHistoryError(Object.assign(new Error('[{"path":["private"],"code":"invalid_value"}]'), { name: "ZodError" }));
  await x.h.value().recover();
  await x.h.settle();
  assert.equal(x.h.value().error, "Conversation history could not be loaded. Please try again.");
  assert.equal(x.h.value().loading, false);
  assert.equal(x.h.value().recoveryRequired, false);
  assert.equal(x.h.value().history?.conversation.id, A);
  x.setHistoryError();
  await x.h.value().recover();
  await x.h.settle();
  assert.equal(x.h.value().error, undefined);
  x.h.unmount();
});

test("private send surfaces definite attachment refusals and consumes after possible admission", async () => {
  for (const [error, admitted, expected] of [
    [buildApiError({ error: "attachment_expired", error_code: 12101, message: "Attachment expired" }, 410), false, "attachment"],
    [buildApiError({ error: "not_found", error_code: 1003, message: "Not found: Attachment not found" }, 404), false, "attachment"],
    [buildApiError({ error: "not_found", error_code: 1003, message: "Not found: Conversation not found" }, 404), false, "refused"],
    [buildApiError({ error: "attachment_expired", error_code: 12101, message: "Attachment expired" }, 410), true, "uncertain"],
    [buildApiError(null, 0), false, "uncertain"],
  ] as const) {
    let attempts = 0;
    const x = await harness(async (input, _event, _controller, accept) => {
      attempts++;
      assert.equal(input.attachmentIds?.length, 1);
      if (admitted) accept?.();
      throw error;
    });
    if (expected === "uncertain") x.setHistoryError(new Error("History connection lost"));
    const send = x.h.value().send("Read this", undefined, {
      conversationId: A,
      attachmentIds: ["11111111-1111-4111-8111-111111111111"],
      attachments: [],
    });
    if (expected === "attachment")
      await assert.rejects(send, (cause) => cause === error);
    else
      assert.equal(await send, expected === "uncertain");
    await x.h.settle();
    assert.equal(x.h.value().sending, false);
    assert.equal(x.h.value().recoveryRequired, expected === "uncertain");
    assert.equal(attempts, 1);
    x.h.unmount();
  }
});

test("sampled streaming measures the first renderable text and freezes duration at the terminal event", async () => {
  let tick = 0;
  const metrics: ChatTurnTimings[] = [];
  const sample = new AssistantTurnPerformance((timings) => metrics.push(timings), () => tick);
  const x = await harness(async (_input, emit, _controller, admitted) => {
    tick = 8;
    admitted?.();
    emit({ event: "turn.status", cursor: 1, conversation_id: A, turn_id: "private-turn", status: "running" });
    emit({ event: "message.started", cursor: 2, message_id: "private-message", role: "assistant" });
    emit({
      event: "block.started", cursor: 3, block_id: "b", index: 0, message_id: "private-message",
      block: { type: "text", block_id: "b", text: "<function_calls>internal data" },
    });
    tick = 15;
    emit({ event: "block.delta", cursor: 4, block_id: "b", text: "</function_calls>" });
    tick = 30;
    emit({ event: "block.delta", cursor: 5, block_id: "b", text: "Private answer" });
    tick = 40;
    emit({ event: "turn.completed", turn_id: "private-turn", cursor: 6, status: "completed", error: null });
    tick = 80;
  });
  await x.h.value().send("Private prompt", undefined, undefined, undefined, undefined, sample);
  assert.equal(metrics.length, 0);
  assert.equal(sample.commitFor("other"), undefined);
  const commit = sample.commitFor("private-turn")!;
  commit();
  commit();
  sample.finish("error");
  assert.deepEqual(metrics, [{ outcome: "completed", headers_ms: 8, first_text_ms: 30, first_text_commit_ms: 80, duration_ms: 40 }]);
  x.h.unmount();
});

test("tool-only completions, transport errors, deadlines and lifecycle cancellation produce one terminal result", async () => {
  const configurations: [ChatTurnTimings["outcome"], typeof streamAssistantTurn][] = [
    ["no_text", async (_input, emit) => {
      emit({ event: "turn.status", cursor: 1, conversation_id: A, turn_id: "turn", status: "running" });
      emit({ event: "turn.completed", cursor: 2, turn_id: "turn", status: "completed", error: null });
    }],
    ["error", async () => { throw new Error("transport failed"); }],
    ["timeout", async (_input, _event, _controller, _admitted, onTimeout) => {
      onTimeout?.();
      throw new Error("transport aborted");
    }],
  ];
  for (const [expected, stream] of configurations) {
    const x = await harness(stream);
    const metrics: ChatTurnTimings[] = [];
    const sample = new AssistantTurnPerformance((value) => metrics.push(value), () => 0);
    await x.h.value().send("secret", undefined, undefined, undefined, undefined, sample);
    assert.deepEqual(metrics, [{ outcome: expected, duration_ms: 0 }]);
    x.h.unmount();
  }
  const x = await harness();
  const metrics: ChatTurnTimings[] = [];
  const sample = new AssistantTurnPerformance((value) => metrics.push(value), () => 0);
  const send = x.h.value().send("secret", undefined, undefined, undefined, undefined, sample);
  x.h.value().openConversation(B);
  x.streams[0]!.done.reject(new Error("request aborted"));
  await send;
  assert.deepEqual(metrics, [{ outcome: "cancelled", headers_ms: 0, duration_ms: 0 }]);
  x.h.unmount();
});

test("a virtualized reply that never commits produces no synthetic commit time", (context) => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  const metrics: ChatTurnTimings[] = [];
  const sample = new AssistantTurnPerformance((value) => metrics.push(value), () => 0);
  const reply = { cursor: 2, turnId: "turn", blocks: [{ id: "b", index: 0, text: "Private answer" }] };
  sample.observe({ event: "turn.status", turn_id: "turn", cursor: 1, conversation_id: A, status: "running" }, reply);
  sample.observe({ event: "block.delta", cursor: 2, block_id: "b", text: "Private answer" }, reply);
  const lateCommit = sample.commitFor("turn")!;
  sample.finish("completed");
  context.mock.timers.tick(CHAT_COMMIT_GRACE_MS);
  lateCommit();
  assert.deepEqual(metrics, [{ outcome: "completed", first_text_ms: 0, duration_ms: 0 }]);
});

test("withdrawing the local chat grant stops clock reads and discards a pending text commit", (context) => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  let reads = 0;
  let unsubscriptions = 0;
  let discard!: () => void;
  const results: ChatTurnTimings[] = [];
  const sample = new AssistantTurnPerformance(
    (result) => results.push(result),
    () => ++reads,
    (listener) => { discard = listener; return () => { unsubscriptions++; }; },
  );
  const live = { cursor: 2, turnId: "t", blocks: [{ id: "b", index: 0, text: "body" }] };
  sample.observe({ event: "turn.status", turn_id: "t", cursor: 1, conversation_id: A, status: "running" }, live);
  sample.observe({ event: "block.delta", cursor: 2, block_id: "b", text: "body" }, live);
  const commit = sample.commitFor("t")!;
  sample.finish("completed");
  discard();
  const previous = reads;
  sample.admitted();
  commit();
  sample.observe({ event: "block.delta", cursor: 3, block_id: "b", text: "later" }, live);
  sample.finish("completed");
  context.mock.timers.tick(CHAT_COMMIT_GRACE_MS);
  assert.equal(reads, previous);
  assert.equal(unsubscriptions, 1);
  assert.deepEqual(results, []);
});
