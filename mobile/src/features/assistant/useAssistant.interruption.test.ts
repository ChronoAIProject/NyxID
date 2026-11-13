import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { deferred, hookHarness } from "./assistantHookTestHarness";
import { historySchema, type AssistantHistory } from "./assistantTypes";
import type { useAssistant } from "./useAssistant";
import type { streamAssistantTurn } from "../../lib/api/assistantApi";

const id = `nyxa-${"a".repeat(32)}`;
const stamp = "2026-11-09T00:00:00Z";
function page(code?: string): AssistantHistory {
  return historySchema.parse({
    conversation: {
      id,
      title: "Question",
      model: "nyxagent/chat",
      created_at: stamp,
      last_message_at: stamp,
      message_count: 2,
      active_turn: null,
    },
    messages: [
      {
        id: "u",
        seq: 1,
        turn_id: "turn",
        role: "user",
        text: "Question",
        status: "completed",
        error_code: null,
        created_at: stamp,
      },
      {
        id: "a",
        seq: 2,
        turn_id: "turn",
        role: "assistant",
        text: "Answer",
        status: code ? "failed" : "completed",
        error_code: code ?? null,
        created_at: stamp,
      },
    ],
    acknowledgements: [],
    approvals: [],
    waiting: [],
    before_seq: null,
  });
}
function running(): AssistantHistory {
  const value = page();
  value.messages.pop();
  value.conversation.active_turn = {
    turn_id: "turn",
    started_at: stamp,
    activities: [],
    attachments: [],
  };
  return value;
}

async function harness(initial?: AssistantHistory) {
  let current = initial ?? { ...page(), messages: [] };
  const done = deferred<void>();
  let app: (state: string) => void = () => {};
  let refresh: (id: string) => Promise<unknown> = async () => {};
  let posts = 0;
  let failHistory = false;
  let stops = 0;
  const agentList = {
    agents: [
      {
        id: "nyxbot",
        kind: "nyxbot",
        can_use: true,
        status: "idle",
        model: "nyxagent/chat",
      },
    ],
  };
  const stream: typeof streamAssistantTurn = async (
    _input,
    onEvent,
    controller,
    accepted,
  ) => {
    posts++;
    accepted?.();
    onEvent({
      event: "turn.status",
      cursor: 1,
      conversation_id: id,
      turn_id: "turn",
      status: "running",
    });
    controller.signal.addEventListener(
      "abort",
      () => done.reject(new Error("Disconnected")),
      { once: true },
    );
    await done.promise;
  };
  const h = hookHarness<ReturnType<typeof useAssistant>>({
    "./useAssistantLiveRefresh": {
      useAssistantLiveRefresh: (
        _active: boolean,
        _id: string,
        _paused: boolean,
        callback: typeof refresh,
      ) => {
        refresh = callback;
      },
    },
    "react-native": {
      AppState: {
        currentState: "active",
        addEventListener: (_: string, callback: typeof app) => {
          app = callback;
          return { remove() {} };
        },
      },
    },
    "@react-navigation/native": { useIsFocused: () => true },
    "@react-native-community/netinfo": { addEventListener: () => () => {} },
    "@tanstack/react-query": {
      useQuery: () => ({ data: agentList, isPending: false, isError: false }),
    },
    "../../lib/api/agentsApi": { agentsApi: {} },
    "../../lib/api/assistantApi": {
      streamAssistantTurn: stream,
      assistantApi: {
        history: async () => {
          if (failHistory) throw new Error("History unavailable");
          return current;
        },
        list: async () => ({ conversations: [], next_cursor: null }),
        stop: async () => {
          stops++;
        },
      },
    },
  });
  const hook = h.load(
    fileURLToPath(new URL("./useAssistant.ts", import.meta.url)),
  ).useAssistant as typeof useAssistant;
  h.mount(() => hook(id));
  await h.settle();
  return {
    h,
    done,
    set: (value: AssistantHistory) => {
      current = value;
    },
    refresh: async () => {
      await refresh(id);
      await h.settle();
    },
    app: (state: string) => {
      app(state);
      h.flush();
    },
    posts: () => posts,
    stops: () => stops,
    failHistory: (value: boolean) => {
      failHistory = value;
    },
    drop: async (next = running()) => {
      const send = h.value().send("Question");
      h.flush();
      current = next;
      done.reject(new Error("SSE disconnected"));
      assert.equal(await send, true); // admitted; don't restore/re-send the draft
      await h.settle();
    },
  };
}

test("final refresh after a dropped stream proves success and removes the interruption", async () => {
  const x = await harness();
  try {
    await x.drop(page());
    assert.equal(x.h.value().error, undefined);
    assert.equal(x.h.value().interrupted, false);
    assert.equal(x.h.value().history?.messages[1]?.status, "completed");
    assert.equal(x.posts(), 1);
  } finally {
    x.h.unmount();
  }
});

for (const code of [
  undefined,
  "assistant_unavailable",
  "cancelled",
  "queue_timeout",
  "turn_lost",
]) {
  test(`late history reconciles ${code ?? "success"}, retaining recorded failure/Stop UI`, async () => {
    const x = await harness();
    try {
      await x.drop();
      assert.match(x.h.value().error ?? "", /may still be working/);
      assert.equal(x.h.value().interrupted, true);
      x.set(page(code));
      await x.refresh();
      assert.equal(x.h.value().interrupted, false);
      if (code === "turn_lost") {
        assert.match(x.h.value().error ?? "", /check which actions completed/);
        assert.doesNotMatch(x.h.value().error ?? "", /may still be working/);
      } else assert.equal(x.h.value().error, undefined);
      assert.equal(x.h.value().history?.messages[1]?.error_code, code ?? null);
      assert.equal(x.posts(), 1);
    } finally {
      x.h.unmount();
    }
    const reload = await harness(page(code));
    try {
      assert.equal(reload.h.value().error, undefined);
      assert.equal(
        reload.h.value().history?.messages[1]?.error_code,
        code ?? null,
      );
    } finally {
      reload.h.unmount();
    }
  });
}

test("idle, unrelated success and failed refresh cannot hide an unknown outcome", async () => {
  const x = await harness();
  try {
    await x.drop();
    x.failHistory(true);
    await assert.rejects(x.refresh(), /History unavailable/);
    assert.match(x.h.value().error ?? "", /may still be working/);
    x.failHistory(false);
    x.set({ ...page(), messages: [] });
    await x.refresh();
    assert.match(x.h.value().error ?? "", /may still be working/);
    x.set({
      ...page(),
      messages: page().messages.map((row) => ({
        ...row,
        id: `other-${row.id}`,
        seq: row.seq + 10,
        turn_id: "other",
      })),
    });
    await x.refresh();
    assert.match(x.h.value().error ?? "", /may still be working/);
  } finally {
    x.h.unmount();
  }
});

for (const code of [undefined, "assistant_unavailable"]) {
  test(`approval wait then continuation on a different stream reconciles ${code ?? "success"}`, async () => {
    const x = await harness();
    try {
      const next = historySchema.parse({
        ...page(),
        messages: [],
        acknowledgements: [
          {
            id: "33333333-3333-4333-8333-333333333333",
            kind: "action",
            creation_turn_id: "turn",
            status: "pending",
            summary: "Publish skill",
            service_name: null,
            service_slug: null,
            tool_name: "nyxid__draft_agent_skill",
            created_at: stamp,
            expires_at: "2099-01-01T00:00:00Z",
            decided_at: null,
          },
        ],
      });
      await x.drop({ ...running(), acknowledgements: next.acknowledgements });
      assert.equal(x.h.value().history?.acknowledgements[0]?.status, "pending");
      x.set(next);
      await x.refresh();
      assert.match(x.h.value().error ?? "", /may still be working/);
      next.acknowledgements[0]!.status = "allowed";
      await x.refresh();
      assert.match(x.h.value().error ?? "", /may still be working/);
      next.messages = page(code).messages.map((row) => ({
        ...row,
        id: `next-${row.id}`,
        seq: row.seq + 10,
        turn_id: "next",
        continuation_of:
          row.role === "user" ? next.acknowledgements[0]!.id : undefined,
      }));
      await x.refresh();
      assert.equal(x.h.value().error, undefined);
      assert.equal(
        x.h.value().history?.messages.at(-1)?.error_code,
        code ?? null,
      );
      assert.equal(x.posts(), 1);
    } finally {
      x.h.unmount();
    }
  });
}

test("background/resume reconciles settlement without resending", async () => {
  const x = await harness();
  try {
    const send = x.h.value().send("Question");
    x.h.flush();
    x.set(running());
    x.app("background");
    await send;
    await x.h.settle();
    assert.match(x.h.value().error ?? "", /may still be working/);
    x.set(page());
    x.app("active");
    await x.h.settle();
    assert.equal(x.h.value().error, undefined);
    assert.equal(x.posts(), 1);
  } finally {
    x.h.unmount();
  }
});

test("Stop acceptance alone keeps the notice; cancelled history settles it", async () => {
  const x = await harness();
  try {
    await x.drop();
    await x.h.value().stop();
    await x.h.settle();
    assert.equal(x.stops(), 1);
    assert.match(x.h.value().error ?? "", /may still be working/);
    x.set(page("cancelled"));
    await x.refresh();
    assert.equal(x.h.value().error, undefined);
    assert.equal(x.h.value().history?.messages[1]?.error_code, "cancelled");
  } finally {
    x.h.unmount();
  }
});
