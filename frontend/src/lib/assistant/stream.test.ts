import { describe, expect, it } from "vitest";
import type { TurnReducerState } from "@/types/assistant";
import { isTurnActive } from "@/types/assistant";
import { applyTurnEvent, EMPTY_TURN_STATE, toTerminalBlock } from "./stream";

describe("applyTurnEvent", () => {
  it("builds a progressively streamed assistant message", () => {
    let state: TurnReducerState = EMPTY_TURN_STATE;
    state = applyTurnEvent(state, {
      cursor: 1,
      event: "turn.status",
      turn_id: "turn-1",
      status: "running",
    });
    state = applyTurnEvent(
      state,
      {
        cursor: 2,
        event: "message.started",
        message_id: "message-1",
        role: "assistant",
      },
      "2026-07-16T00:00:00.000Z",
    );
    state = applyTurnEvent(state, {
      cursor: 3,
      event: "block.started",
      message_id: "message-1",
      block_id: "block-1",
      index: 0,
      block: { type: "text", block_id: "block-1", text: "" },
    });
    state = applyTurnEvent(state, {
      cursor: 4,
      event: "block.delta",
      block_id: "block-1",
      text: "Brokered ",
    });
    state = applyTurnEvent(state, {
      cursor: 5,
      event: "block.delta",
      block_id: "block-1",
      text: "and scoped.",
    });

    expect(state.activeTurn?.status).toBe("running");
    expect(state.messages[0]?.created_at).toBe("2026-07-16T00:00:00.000Z");
    expect(state.messages[0]?.blocks[0]).toEqual({
      type: "text",
      block_id: "block-1",
      text: "Brokered and scoped.",
    });
    expect(state.lastCursor).toBe(5);
  });

  it("drops duplicate and out-of-order cursor delivery", () => {
    const initial = applyTurnEvent(EMPTY_TURN_STATE, {
      cursor: 3,
      event: "turn.status",
      turn_id: "turn-1",
      status: "running",
    });
    const duplicate = applyTurnEvent(initial, {
      cursor: 3,
      event: "turn.status",
      turn_id: "turn-1",
      status: "waiting",
    });
    const older = applyTurnEvent(initial, {
      cursor: 2,
      event: "turn.status",
      turn_id: "turn-1",
      status: "cancelled",
    });

    expect(duplicate).toBe(initial);
    expect(older).toBe(initial);
    expect(initial.activeTurn?.status).toBe("running");
  });

  it("stores blocked as a terminal turn state", () => {
    const running = applyTurnEvent(EMPTY_TURN_STATE, {
      cursor: 1,
      event: "turn.status",
      turn_id: "turn-blocked",
      status: "running",
    });
    const blocked = applyTurnEvent(running, {
      cursor: 2,
      event: "turn.completed",
      turn_id: "turn-blocked",
      status: "blocked",
      error: null,
    });

    expect(blocked.activeTurn).toEqual({
      turnId: "turn-blocked",
      status: "blocked",
      error: null,
    });
    expect(isTurnActive(blocked.activeTurn?.status)).toBe(false);
  });

  it("uses whole-field replacement semantics for block patches", () => {
    const message = {
      id: "message-1",
      role: "assistant" as const,
      schema_version: 1,
      created_at: "2026-07-16T00:00:00.000Z",
      blocks: [
        {
          type: "run" as const,
          block_id: "run-1",
          title: "RUN",
          steps_total: 2,
          steps_complete: 0,
          state: "running" as const,
          steps: [
            {
              index: 1,
              status: "active" as const,
              label: "First",
              meta: "Working",
              service_slug: null,
              artifact_id: null,
              approval_request_id: null,
            },
            {
              index: 2,
              status: "waiting" as const,
              label: "Second",
              meta: "Queued",
              service_slug: null,
              artifact_id: null,
              approval_request_id: null,
            },
          ],
        },
      ],
    };
    const state = applyTurnEvent(
      { messages: [message], activeTurn: null, lastCursor: 0 },
      {
        cursor: 1,
        event: "block.updated",
        block_id: "run-1",
        patch: {
          steps_complete: 1,
          steps: [
            {
              index: 1,
              status: "done",
              label: "First",
              meta: "Complete",
              service_slug: null,
              artifact_id: null,
              approval_request_id: null,
            },
          ],
        },
      },
    );

    const block = state.messages[0]?.blocks[0];
    expect(block?.type).toBe("run");
    if (block?.type === "run") {
      expect(block.steps).toHaveLength(1);
      expect(block.steps_complete).toBe(1);
    }
  });
});

describe("toTerminalBlock", () => {
  it("cancels an in-flight run and skips its open steps", () => {
    const block = toTerminalBlock({
      type: "run",
      block_id: "run-1",
      title: "RUN",
      steps_total: 2,
      steps_complete: 1,
      state: "awaiting_approval",
      steps: [
        {
          index: 1,
          status: "done",
          label: "read",
          meta: "done",
          service_slug: null,
          artifact_id: null,
          approval_request_id: null,
        },
        {
          index: 2,
          status: "waiting",
          label: "write",
          meta: "waiting for approval",
          service_slug: null,
          artifact_id: null,
          approval_request_id: "approval-1",
        },
      ],
    });
    expect(block).toMatchObject({
      state: "cancelled",
      steps: [{ status: "done" }, { status: "skipped" }],
    });
  });

  it("marks a pending approval as cancelled and times out a waiting connect card", () => {
    const approval = toTerminalBlock({
      type: "approval_card",
      block_id: "approval-1",
      approval_request_id: "approval-1",
      body: "b",
      service_slug: "s",
      agent_key_prefix: "nyxid_ag_...1",
      approval_mode: "per_request",
      grant_duration_sec: null,
      expires_at: "2026-07-16T00:10:00.000Z",
      decision: null,
      decision_channel: null,
    });
    expect(approval).toMatchObject({ decision: "cancelled" });

    const connect = toTerminalBlock({
      type: "connect_card",
      block_id: "connect-1",
      catalog_slug: "github",
      service_name: "GitHub",
      icon_url: "https://cdn.nyxid.dev/catalog/github.svg",
      subtitle: "s",
      auth_kind: "oauth",
      requested_scopes: [],
      key_id: null,
      granted_scopes: null,
      device_user_code: null,
      device_verification_url: null,
      state: "waiting_for_user",
      error_message: null,
      steps: [],
      footer: "f",
    });
    expect(connect).toMatchObject({ state: "timed_out" });
  });

  it("passes terminal and immutable blocks through unchanged", () => {
    const decided = {
      type: "approval_card",
      block_id: "approval-2",
      approval_request_id: "approval-2",
      body: "b",
      service_slug: "s",
      agent_key_prefix: "nyxid_ag_...2",
      approval_mode: "per_request",
      grant_duration_sec: null,
      expires_at: "2026-07-16T00:10:00.000Z",
      decision: "approved",
      decision_channel: "web",
    } as const;
    expect(toTerminalBlock(decided)).toBe(decided);
    const text = { type: "text", block_id: "text-1", text: "hello" } as const;
    expect(toTerminalBlock(text)).toBe(text);
  });

  it("keeps a pending action card interactive when its turn is cancelled", () => {
    const action = {
      type: "action_card",
      block_id: "action-1",
      action: "service.connect",
      action_request_id: "act-1",
      origin_turn_id: "turn-1",
      params: {
        variant: "catalog",
        service_slug: "api-github",
        requested_scopes: ["repo"],
        via_node_id: null,
        target_org_id: null,
      },
      status: "pending",
      outcome_note: "",
    } as const;

    expect(toTerminalBlock(action)).toBe(action);
  });
});
