import { expect, it } from "vitest";
import type { CmaTranscriptTurn } from "@/schemas/assistant-cma-agent";
import type { CmaDisplayEvent } from "./cma-events";
import {
  cmaTurnEvents,
  cmaTurnStatus,
  linkedCmaTurn,
} from "./cma-conversation";

const turn = (
  sequence: number,
  status: CmaTranscriptTurn["status"],
): CmaTranscriptTurn => ({
  turn_sequence: sequence,
  started_at: null,
  status,
  messages: [],
});
const event = (
  sequence: number,
  patch: Partial<CmaDisplayEvent> = {},
): CmaDisplayEvent => ({
  key: String(sequence),
  sequence,
  type: "status_changed",
  message: "",
  ...patch,
});

it("locates the linked turn instead of the newest request", () => {
  const events = [
    event(10, { type: "turn_started", turnSequence: 10, responseId: "old" }),
    event(20, { type: "turn_started", turnSequence: 20, responseId: "new" }),
  ];
  expect(linkedCmaTurn(events, "old")).toBe(10);
  expect(linkedCmaTurn(events, "missing")).toBeUndefined();
  expect(linkedCmaTurn(events, undefined)).toBeUndefined();
});
it("rejects conflicting or incomplete response bindings", () => {
  expect(
    linkedCmaTurn(
      [
        event(10, {
          type: "turn_started",
          turnSequence: 10,
          responseId: "same",
        }),
        event(20, {
          type: "turn_started",
          turnSequence: 20,
          responseId: "same",
        }),
      ],
      "same",
    ),
  ).toBeUndefined();
  expect(
    linkedCmaTurn(
      [event(12, { responseId: "same", turnSequence: 10 })],
      "same",
    ),
  ).toBeUndefined();
});
it("keeps newer approvals and explicit foreign bindings out of an old turn", () => {
  const old = turn(10, "completed");
  const events = cmaTurnEvents(
    old,
    [old, turn(20, "active")],
    [
      event(10, { type: "turn_started", turnSequence: 10, responseId: "old" }),
      event(12, { type: "tool_updated", message: "old tool" }),
      event(15, { turnSequence: 20, status: "waiting_approval" }),
      event(16, { responseId: "foreign", status: "waiting_approval" }),
      event(22, { status: "waiting_approval" }),
    ],
  );
  expect(events.map((item) => item.sequence)).toEqual([10, 12]);
  expect(cmaTurnStatus(old, events)).toBe("completed");
});
it("shows an active turn's observed approval wait without promoting ready to success", () => {
  expect(
    cmaTurnStatus(turn(20, "active"), [
      event(22, { status: "waiting_approval" }),
    ]),
  ).toBe("waiting_approval");
  expect(
    cmaTurnStatus(turn(20, "active"), [event(23, { status: "ready" })]),
  ).toBe("in_progress");
  expect(cmaTurnStatus(turn(20, "interrupted"), [])).toBe("interrupted");
});
