import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { deferred, hookHarness } from "./assistantHookTestHarness";
import { historySchema, type AssistantHistory } from "./assistantTypes";
import type { useAssistant } from "./useAssistant";
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

async function harness() {
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
      streamAssistantTurn: stream,
      assistantApi: {
        history: async (id: string) => {
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
    x.streams[0].done.resolve();
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
  x.streams[0].done.resolve();
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
  assert.equal(x.streams[1].controller.signal.aborted, false);
  await assert.rejects(x.h.value().send("third message"), /already being sent/);
  assert.equal(x.streams.length, 2);
  x.streams[1].done.resolve();
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
  x.streams[0].done.resolve();
  await x.h.settle();
  x.h.unmount();
  x.release();
  await send;
  assert.equal(x.streams.length, 1);
});
