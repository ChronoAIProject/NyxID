import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import test from "node:test";
import fixture from "../../../../shared/agent-avatars/mood-fixture.json";
import { hookHarness } from "./assistantHookTestHarness";
import { groupMemberMood, moodInputFor, settledReplyOf } from "./agentMood";
import type { AssistantMessage } from "./assistantTypes";
import type { AvatarSettledReply } from "../../lib/avatars/agentAvatars";

const message = (id: string, extra: Partial<AssistantMessage>) =>
  ({
    id,
    role: "assistant",
    status: "completed",
    error_code: null,
    ...extra,
  }) as AssistantMessage;

test("settled replies: completions, failures and stops (cancelled)", () => {
  assert.equal(settledReplyOf(undefined), null);
  assert.equal(settledReplyOf([message("u", { role: "user" })]), null);
  assert.deepEqual(
    settledReplyOf([message("a", {}), message("u", { role: "user" })]),
    {
      id: "a",
      outcome: "completed",
    },
  );
  assert.deepEqual(
    settledReplyOf([
      message("b", { status: "failed", error_code: "upstream_error" }),
    ]),
    {
      id: "b",
      outcome: "failed",
    },
  );
  // A stopped turn is a failure with error_code "cancelled": it never reads as oops.
  assert.deepEqual(
    settledReplyOf([
      message("c", { status: "failed", error_code: "cancelled" }),
    ]),
    {
      id: "c",
      outcome: "stopped",
    },
  );
});

test("the chat screen's turn state maps to mood input", () => {
  assert.deepEqual(
    moodInputFor({
      activeTurn: null,
      sending: false,
      pendingOwnerAction: true,
    }),
    { activeTurn: null, pendingOwnerAction: true },
  );
  const sending = moodInputFor({
    activeTurn: null,
    sending: true,
    liveText: "Hi",
    pendingOwnerAction: false,
  });
  assert.equal(sending.activeTurn?.replyStreaming, true);
  const turn = moodInputFor({
    activeTurn: {
      turn_id: "t",
      started_at: "2026-11-16T00:00:00Z",
      stop_requested: true,
      queued: false,
      continuations: 1,
      activities: [{ status: "error" }, { status: "running" }],
      attachments: [],
    } as never,
    sending: false,
    pendingOwnerAction: false,
  });
  assert.deepEqual(turn.activeTurn?.activityStatuses, ["error", "running"]);
  assert.equal(turn.activeTurn?.stopRequested, true);
  assert.equal(turn.activeTurn?.continuations, 1);
});

test("a working group member thinks, or asks while its card waits", () => {
  assert.equal(groupMemberMood("a", []), "thinking");
  assert.equal(groupMemberMood("a", [{ agent_id: "b" }]), "thinking");
  assert.equal(groupMemberMood("a", [{ agent_id: "a" }]), "approval");
});

function moodHook() {
  const timers: (() => void)[] = [];
  const harness = hookHarness<string>(
    {},
    {
      setTimeout: (callback: () => void) => {
        timers.push(callback);
        return timers.length;
      },
      clearTimeout: (id: number) => {
        timers[id - 1] = () => {};
      },
    },
  );
  const { useAgentMood } = harness.load(
    fileURLToPath(new URL("./agentMood.ts", import.meta.url)),
  ) as typeof import("./agentMood");
  let state = {
    conversation: "c",
    active: false,
    reply: null as AvatarSettledReply | null,
  };
  const render = () =>
    useAgentMood(
      state.conversation,
      { activeTurn: state.active ? {} : null, pendingOwnerAction: false },
      state.reply,
    );
  return {
    mount: () => harness.mount(render),
    set(next: Partial<typeof state>) {
      state = { ...state, ...next };
    },
    step(next: Partial<typeof state>) {
      state = { ...state, ...next };
      return harness.render();
    },
    expire() {
      timers.splice(0).forEach((timer) => timer());
      return harness.flush();
    },
  };
}

test("the hook runs every shared settlement sequence", () => {
  for (const sequence of fixture.sequences) {
    const hook = moodHook();
    let mounted = false;
    for (const step of sequence.steps) {
      const next = {
        conversation:
          "conversation" in step ? (step.conversation as string) : "c",
        active: step.active,
        reply: step.reply as AvatarSettledReply | null,
      };
      let mood: string;
      if (!mounted) {
        hook.set(next);
        mood = hook.mount();
        mounted = true;
      } else if ("expire" in step && step.expire) {
        hook.set(next);
        mood = hook.expire();
      } else {
        mood = hook.step(next);
      }
      assert.equal(
        mood,
        step.mood,
        `${sequence.name}: ${JSON.stringify(step)}`,
      );
    }
  }
});

test("a stop never flashes happy, and success celebrates only briefly", () => {
  const hook = moodHook();
  hook.set({ reply: { id: "r1", outcome: "completed" } });
  assert.equal(hook.mount(), "idle");
  assert.equal(hook.step({ active: true }), "thinking");
  // The turn clears before its reply row arrives: neutral, not happy.
  assert.equal(hook.step({ active: false }), "idle");
  assert.equal(hook.step({ reply: { id: "r2", outcome: "stopped" } }), "idle");
  assert.equal(hook.step({ active: true }), "thinking");
  assert.equal(
    hook.step({ active: false, reply: { id: "r3", outcome: "completed" } }),
    "success",
  );
  assert.equal(hook.expire(), "idle");
});
