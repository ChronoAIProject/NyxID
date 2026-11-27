import { expect, it } from "vitest";
import { voicePhaseTimingSchema, voiceSnapshotSchema } from "./assistant-voice";

const report = {
  unit: "ms",
  offset_origin: "call_created_at",
  measurements: [
    {
      phase: "playback_confirmed",
      source: "client_playback",
      kind: "offset",
      value_ms: 1250,
    },
  ],
  unavailable: [
    "asr_processing",
    "synthesis_processing",
    "device_playback",
    "first_audio",
  ],
};
const snapshot = {
  type: "snapshot",
  session: {
    id: "00000000-0000-4000-8000-000000000001",
    created_at: "2026-11-27T00:00:00Z",
    closed_at: null,
    generation: 1,
    state: "active",
    muted: false,
    input_muted: false,
    end_requested: false,
    control_revision: 0,
    observed_seconds: 1,
    reserved_until: 30,
    final_usage_confirmed: false,
    end_reason: null,
    idle_warning: false,
  },
  captions: [],
  tasks: [
    {
      id: "00000000-0000-4000-8000-000000000002",
      turn_id: "local-turn",
      state: "claimed",
      pending_acknowledgement_ids: [],
    },
  ],
};

it("reads legacy snapshots and isolates malformed timing without dropping tasks or audio control", () => {
  expect(voiceSnapshotSchema.parse(snapshot).tasks).toHaveLength(1);
  const parsed = voiceSnapshotSchema.parse({
    ...snapshot,
    tasks: [
      { ...snapshot.tasks[0], phase_timing: { ...report, unit: "seconds" } },
    ],
    turn_timings: [
      null,
      { turn_id: "provider-id", kind: "output", phase_timing: report },
      {
        turn_id: "00000000-0000-4000-8000-000000000003",
        kind: "output",
        phase_timing: report,
      },
    ],
  });
  expect(parsed.session.state).toBe("active");
  expect(parsed.tasks).toHaveLength(1);
  expect(parsed.tasks[0]?.phase_timing).toBeNull();
  expect(parsed.turn_timings).toHaveLength(1);
});

it("keeps units, source and unavailable stages explicit and rejects unbounded measurements", () => {
  const parsed = voicePhaseTimingSchema.parse(report);
  expect(parsed.measurements[0]).toEqual(report.measurements[0]);
  expect(parsed.measurements.some((m) => m.phase === "first_audio")).toBe(
    false,
  );
  for (const measurement of [
    { ...report.measurements[0], value_ms: -1 },
    { ...report.measurements[0], value_ms: 86_400_001 },
    { ...report.measurements[0], source: "private-provider-id" },
    { ...report.measurements[0], phase: "raw-text" },
    { ...report.measurements[0], tool_index: 40 },
  ]) {
    expect(
      voicePhaseTimingSchema.safeParse({
        ...report,
        measurements: [measurement],
      }).success,
    ).toBe(false);
  }
  expect(
    voicePhaseTimingSchema.safeParse({
      ...report,
      measurements: Array(129).fill(report.measurements[0]),
    }).success,
  ).toBe(false);
});
