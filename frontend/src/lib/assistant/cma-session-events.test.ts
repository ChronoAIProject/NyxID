import { describe, expect, it } from "vitest";
import {
  applyCmaEvent,
  CmaSseDecoder,
  cmaSessionStatus,
} from "./cma-session-events";
import { cmaEvent } from "./cma-session.test-fixtures";
import type { CmaSessionState } from "@/schemas/cma-session";

const initial: CmaSessionState = {
  title: "Task",
  status: "working",
  connection: "live",
  actions: [],
};

describe("CMA public event projection", () => {
  it("reconciles tool updates and does not invent action success at turn end", () => {
    let state = applyCmaEvent(
      initial,
      cmaEvent(11, {
        type: "tool_started",
        call_id: "a",
        title: "Inspecting the page",
        kind: "browser",
        status: "in_progress",
      }),
    );
    state = applyCmaEvent(
      state,
      cmaEvent(12, {
        type: "tool_started",
        call_id: "b",
        title: "Checking the output",
        status: "in_progress",
      }),
    );
    state = applyCmaEvent(
      state,
      cmaEvent(13, {
        type: "tool_updated",
        call_id: "a",
        title: null,
        status: "completed",
      }),
    );
    state = applyCmaEvent(
      state,
      cmaEvent(14, { type: "turn_completed", turn_sequence: 10 }),
    );
    expect(state.status).toBe("completed");
    expect(state.actions).toEqual([
      expect.objectContaining({
        id: "a",
        title: "Inspecting the page",
        status: "completed",
      }),
      expect.objectContaining({ id: "b", status: "unknown" }),
    ]);
    state = applyCmaEvent(
      state,
      cmaEvent(15, { type: "turn_started", turn_sequence: 15 }),
    );
    expect(state).toMatchObject({ status: "working", actions: [] });
  });

  it("handles approval, pause, and additive or withheld events", () => {
    const waiting = applyCmaEvent(
      initial,
      cmaEvent(11, { type: "approval_requested", approval_id: "approval-1" }),
    );
    expect(waiting).toMatchObject({
      status: "waiting",
      notice: "CMA is waiting for approval.",
    });
    expect(cmaSessionStatus("waiting_approval")).toBe("waiting");
    expect(
      applyCmaEvent(
        waiting,
        cmaEvent(12, { type: "status_changed", from: "working", to: "paused" }),
      ),
    ).toMatchObject({ status: "paused", notice: undefined });
    expect(applyCmaEvent(initial, cmaEvent(13, null))).toBe(initial);
    expect(applyCmaEvent(initial, cmaEvent(14, { type: "future_event" }))).toBe(
      initial,
    );
    expect(
      applyCmaEvent(
        initial,
        cmaEvent(15, { type: "message", content: "Private message text" }),
      ),
    ).toBe(initial);
  });

  it("bounds displayed history and preserves the final title of each call", () => {
    let state = initial;
    for (let i = 1; i <= 40; i++) {
      state = applyCmaEvent(
        state,
        cmaEvent(i, {
          type: "tool_started",
          call_id: String(i),
          title: "Inspect",
          status: "in_progress",
        }),
      );
    }
    state = applyCmaEvent(
      state,
      cmaEvent(41, {
        type: "tool_updated",
        call_id: "40",
        title: "Inspected",
        status: "completed",
      }),
    );
    expect(state.actions).toHaveLength(30);
    expect(state.actions[0]?.id).toBe("11");
    expect(state.actions.at(-1)).toMatchObject({
      id: "40",
      title: "Inspected",
      status: "completed",
    });
  });
});

describe("CMA SSE framing", () => {
  it("waits for complete frames across CRLF boundaries and joins data lines", () => {
    const decoder = new CmaSseDecoder();
    expect(
      decoder.push(
        ": heartbeat\r\n\r\nevent: cma.agent.event\r\nid: cursor-1\r\ndata: {\r",
      ),
    ).toEqual([]);
    expect(decoder.push('\ndata: "kind":null}\r\n\r')).toEqual([]);
    expect(decoder.push("\n")).toEqual([
      { event: "cma.agent.event", id: "cursor-1", data: '{\n"kind":null}' },
    ]);
    expect(decoder.push("data: one\n\ndata: two\n\n")).toEqual([
      { event: "message", data: "one" },
      { event: "message", data: "two" },
    ]);
  });

  it("bounds incomplete frames", () => {
    expect(() =>
      new CmaSseDecoder().push("x".repeat(4 * 1024 * 1024 + 1)),
    ).toThrow("too large");
  });
});
