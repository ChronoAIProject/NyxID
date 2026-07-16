import { beforeEach, describe, expect, it } from "vitest";
import { MockAssistantStore } from "./mock-data";

const FIXED_NOW = Date.parse("2026-07-16T00:00:00.000Z");

function findRunStep(store: MockAssistantStore) {
  const history = store.getHistory("conversation-stripe");
  for (const message of history.messages) {
    for (const block of message.blocks) {
      if (block.type === "run") {
        const step = block.steps.find(
          (candidate) => candidate.approval_request_id !== null,
        );
        if (step) return { run: block, step };
      }
    }
  }
  throw new Error("Approval-gated run step not found in showcase data.");
}

describe("MockAssistantStore.decideApproval", () => {
  let store: MockAssistantStore;

  beforeEach(() => {
    store = new MockAssistantStore();
    store.reset(() => FIXED_NOW);
  });

  it("completes the gated step and refreshes its meta on approve", () => {
    store.decideApproval("conversation-stripe", "approval-stripe-lark", true);
    const { run, step } = findRunStep(store);
    expect(run.state).toBe("completed");
    expect(run.steps_complete).toBe(run.steps_total);
    expect(step.status).toBe("done");
    expect(step.meta).not.toMatch(/waiting for approval/i);
    expect(step.meta).toMatch(/approved/i);
  });

  it("fails the gated step and refreshes its meta on deny", () => {
    store.decideApproval("conversation-stripe", "approval-stripe-lark", false);
    const { run, step } = findRunStep(store);
    expect(run.state).toBe("failed");
    expect(step.status).toBe("failed");
    expect(step.meta).not.toMatch(/waiting for approval/i);
    expect(step.meta).toMatch(/denied/i);
  });
});

describe("MockAssistantStore.decideApproval scoping", () => {
  it("leaves runs gated by a different approval request untouched", () => {
    const store = new MockAssistantStore();
    store.reset(() => FIXED_NOW);
    // Second gated run + card in the same conversation, different request id.
    store.appendUserMessage("conversation-stripe", "Also gate a second write.");
    store.applyEvent("conversation-stripe", {
      cursor: 1,
      event: "message.started",
      message_id: "message-second-gate",
      role: "assistant",
    });
    store.applyEvent("conversation-stripe", {
      cursor: 2,
      event: "block.started",
      message_id: "message-second-gate",
      block_id: "run-second",
      index: 0,
      block: {
        type: "run",
        block_id: "run-second",
        title: "RUN",
        steps_total: 1,
        steps_complete: 0,
        state: "awaiting_approval",
        steps: [
          {
            index: 1,
            status: "waiting",
            label: "telegram-bot - send alert",
            meta: "waiting for approval",
            service_slug: "telegram-bot",
            artifact_id: null,
            approval_request_id: "approval-request-telegram-1",
          },
        ],
      },
    });
    store.applyEvent("conversation-stripe", {
      cursor: 3,
      event: "block.started",
      message_id: "message-second-gate",
      block_id: "approval-telegram",
      index: 1,
      block: {
        type: "approval_card",
        block_id: "approval-telegram",
        approval_request_id: "approval-request-telegram-1",
        body: "Send the alert to Telegram.",
        service_slug: "telegram-bot",
        agent_key_prefix: "nyxid_ag_...7f3d",
        approval_mode: "per_request",
        grant_duration_sec: null,
        expires_at: new Date(FIXED_NOW + 60_000).toISOString(),
        decision: null,
        decision_channel: null,
      },
    });

    store.decideApproval("conversation-stripe", "approval-stripe-lark", true);

    const history = store.getHistory("conversation-stripe");
    const blocks = history.messages.flatMap((message) => message.blocks);
    const untouchedRun = blocks.find(
      (block) => block.type === "run" && block.block_id === "run-second",
    );
    const untouchedCard = blocks.find(
      (block) =>
        block.type === "approval_card" &&
        block.block_id === "approval-telegram",
    );
    expect(untouchedRun).toMatchObject({
      state: "awaiting_approval",
      steps_complete: 0,
    });
    expect(untouchedCard).toMatchObject({ decision: null });
    const decidedRun = blocks.find(
      (block) => block.type === "run" && block.block_id === "run-stripe-digest",
    );
    expect(decidedRun).toMatchObject({ state: "completed" });
  });
});

describe("MockAssistantStore.decideApproval same-run multi-gate", () => {
  function seedTwoGatedSteps(store: MockAssistantStore) {
    store.appendUserMessage("conversation-stripe", "Two gated writes, one run.");
    store.applyEvent("conversation-stripe", {
      cursor: 1,
      event: "message.started",
      message_id: "message-multi-gate",
      role: "assistant",
    });
    store.applyEvent("conversation-stripe", {
      cursor: 2,
      event: "block.started",
      message_id: "message-multi-gate",
      block_id: "run-multi",
      index: 0,
      block: {
        type: "run",
        block_id: "run-multi",
        title: "RUN",
        steps_total: 3,
        steps_complete: 1,
        state: "awaiting_approval",
        steps: [
          {
            index: 1,
            status: "done",
            label: "read data",
            meta: "done",
            service_slug: null,
            artifact_id: null,
            approval_request_id: null,
          },
          {
            index: 2,
            status: "waiting",
            label: "write A",
            meta: "waiting for approval",
            service_slug: "lark-bot",
            artifact_id: null,
            approval_request_id: "req-a",
          },
          {
            index: 3,
            status: "waiting",
            label: "write B",
            meta: "waiting for approval",
            service_slug: "telegram-bot",
            artifact_id: null,
            approval_request_id: "req-b",
          },
        ],
      },
    });
    for (const [cursor, blockId, requestId] of [
      [3, "card-a", "req-a"],
      [4, "card-b", "req-b"],
    ] as const) {
      store.applyEvent("conversation-stripe", {
        cursor,
        event: "block.started",
        message_id: "message-multi-gate",
        block_id: blockId,
        index: cursor - 2,
        block: {
          type: "approval_card",
          block_id: blockId,
          approval_request_id: requestId,
          body: `Approve ${requestId}.`,
          service_slug: "svc",
          agent_key_prefix: "nyxid_ag_...7f3d",
          approval_mode: "per_request",
          grant_duration_sec: null,
          expires_at: new Date(FIXED_NOW + 60_000).toISOString(),
          decision: null,
          decision_channel: null,
        },
      });
    }
  }

  function getRun(store: MockAssistantStore) {
    const run = store.findBlock("conversation-stripe", "run-multi");
    if (run?.type !== "run") throw new Error("run-multi not found");
    return run;
  }

  it("keeps the run open until every gated step is decided", () => {
    const store = new MockAssistantStore();
    store.reset(() => FIXED_NOW);
    seedTwoGatedSteps(store);

    store.decideApproval("conversation-stripe", "card-a", true);
    let run = getRun(store);
    expect(run.state).toBe("awaiting_approval");
    expect(run.steps_complete).toBe(2);
    expect(run.steps[2]?.status).toBe("waiting");

    store.decideApproval("conversation-stripe", "card-b", true);
    run = getRun(store);
    expect(run.state).toBe("completed");
    expect(run.steps_complete).toBe(3);
  });

  it("deny fails the run terminally, skips open steps, and cancels their cards", () => {
    const store = new MockAssistantStore();
    store.reset(() => FIXED_NOW);
    seedTwoGatedSteps(store);

    store.decideApproval("conversation-stripe", "card-a", false);

    let run = getRun(store);
    expect(run.state).toBe("failed");
    expect(run.steps[1]?.status).toBe("failed");
    expect(run.steps[2]?.status).toBe("skipped");
    expect(run.steps_complete).toBe(1);
    const cardB = store.findBlock("conversation-stripe", "card-b");
    expect(cardB).toMatchObject({
      decision: "cancelled",
      decision_channel: null,
    });

    // Cancelled ≠ pending: only the showcase stripe card is still waiting.
    expect(store.workspaceCounts().pendingApprovals).toBe(1);

    // The failed run is terminal — deciding the other gate afterwards must
    // neither resurrect it nor flip the cancelled card.
    expect(() =>
      store.decideApproval("conversation-stripe", "card-b", true),
    ).toThrow(/already decided/i);
    run = getRun(store);
    expect(run.state).toBe("failed");
    expect(run.steps[2]?.status).toBe("skipped");
    expect(store.findBlock("conversation-stripe", "card-b")).toMatchObject({
      decision: "cancelled",
    });
  });
});

describe("MockAssistantStore.listApprovals", () => {
  it("seeds decided history and orders pending first, then most recent", () => {
    const store = new MockAssistantStore();
    store.reset(() => FIXED_NOW);
    const entries = store.listApprovals();
    expect(entries.map((entry) => entry.block.block_id)).toEqual([
      "approval-stripe-lark",
      "approval-github-rotate",
      "approval-weekly-lark",
      "approval-weekly-email",
    ]);
    expect(entries[0]?.block.decision).toBeNull();
    expect(entries[1]?.block).toMatchObject({
      decision: "approved",
      decision_channel: "web",
      service_slug: "github",
    });
    expect(entries[2]?.block).toMatchObject({
      decision: "expired",
      decision_channel: null,
    });
    expect(entries[3]?.block).toMatchObject({
      decision: "denied",
      decision_channel: "mobile",
    });
    for (const entry of entries) {
      expect(Number.isNaN(Date.parse(entry.requestedAt))).toBe(false);
    }
  });
});

describe("MockAssistantStore.workspaceCounts", () => {
  it("counts artifacts and pending approvals across conversations", () => {
    const store = new MockAssistantStore();
    store.reset(() => FIXED_NOW);
    expect(store.workspaceCounts()).toEqual({
      artifacts: 2,
      pendingApprovals: 1,
    });
  });

  it("drops the pending approval count once decided", () => {
    const store = new MockAssistantStore();
    store.reset(() => FIXED_NOW);
    store.decideApproval("conversation-stripe", "approval-stripe-lark", true);
    expect(store.workspaceCounts().pendingApprovals).toBe(0);
  });
});
