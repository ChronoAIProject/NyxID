import { render, screen } from "@testing-library/react";
import { expect, it } from "vitest";
import { voicePhaseTimingSchema } from "@/schemas/assistant-voice";
import { VoicePhaseTimings } from "./voice-phase-timing";

it("distinguishes durations, call offsets, sources and unobservable physical playback", () => {
  render(
    <VoicePhaseTimings
      report={voicePhaseTimingSchema.parse({
        unit: "ms",
        offset_origin: "call_created_at",
        measurements: [
          {
            phase: "model",
            source: "nyxagent",
            kind: "duration",
            value_ms: 200,
          },
          {
            phase: "playback_confirmed",
            source: "client_playback",
            kind: "offset",
            value_ms: 3000,
          },
        ],
        unavailable: [
          "asr_processing",
          "synthesis_processing",
          "first_audio",
          "device_playback",
        ],
      })}
    />,
  );
  expect(screen.getByText("200 ms duration")).toBeInTheDocument();
  expect(screen.getByText("3000 ms offset")).toBeInTheDocument();
  expect(screen.getByText("Client playback confirmed")).toBeInTheDocument();
  expect(screen.getByText(/Unavailable phases:/)).toHaveTextContent(
    "Physical device playback",
  );
  expect(
    screen.getByText(/Offsets are milliseconds from call creation/),
  ).toBeInTheDocument();
  expect(
    screen.getByText(
      /Client playback feedback does not measure physical hearing/,
    ),
  ).toBeInTheDocument();
});
