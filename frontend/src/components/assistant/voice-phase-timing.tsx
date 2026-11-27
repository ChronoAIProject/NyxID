import { translateUi } from "@/i18n/use-ui-translation";
import type { VoicePhaseTiming } from "@/schemas/assistant-voice";

const phaseLabels = {
  input_start: "Input start",
  input_end: "Input end",
  transcript_observed: "Transcript observed",
  delegation: "Delegation observed",
  queued: "Queue admission",
  first_activity: "First activity",
  tool_start: "Tool start",
  tool_end: "Tool end",
  first_progress: "First progress",
  approval_wait: "Approval wait began",
  result: "Result ready",
  playback_confirmed: "Client playback confirmed",
  model_start: "Model start",
  model_end: "Model end",
  model: "Model processing",
  tool: "Tool processing",
  total: "Total execution",
  output_start: "Output start",
  output_end: "Output end",
  response_start: "Response start",
  response_end: "Response end",
  response_generation: "Response generation",
  first_audio: "First provider audio",
  mcp_round_trip: "MCP round trip",
  transcription_turnaround: "Transcription turnaround",
  asr_processing: "ASR processing",
  synthesis_processing: "Speech synthesis processing",
  device_playback: "Physical device playback",
  client_playback: "Client playback feedback",
};
const sourceLabels = {
  coordinator: "Server observation",
  provider_transcript: "Provider transcript",
  relay_audio: "Audio relay",
  native_responses: "Native Responses",
  nyxagent: "Task runner",
  mcp_client: "MCP client",
  client_playback: "Client playback feedback",
};

export function VoicePhaseTimings({ report }: { report: VoicePhaseTiming }) {
  return (
    <details className="mt-1 text-12 text-text-secondary">
      <summary className="cursor-pointer">
        {translateUi("Phase timings")}
      </summary>
      <p>
        {translateUi(
          "Offsets are milliseconds from call creation. Durations measure an observed interval.",
        )}
      </p>
      <table className="my-1 w-full text-left">
        <thead>
          <tr>
            <th>{translateUi("Phase")}</th>
            <th>{translateUi("Measurement")}</th>
            <th>{translateUi("Source")}</th>
          </tr>
        </thead>
        <tbody>
          {report.measurements.map((m, i) => (
            <tr key={i}>
              <td>
                {m.tool_index === undefined
                  ? translateUi(phaseLabels[m.phase])
                  : translateUi("Tool {{number}}: {{phase}}", {
                      number: m.tool_index + 1,
                      phase: translateUi(phaseLabels[m.phase]),
                    })}
              </td>
              <td className="whitespace-nowrap font-mono">
                {m.kind === "offset"
                  ? translateUi("{{value}} ms offset", { value: m.value_ms })
                  : translateUi("{{value}} ms duration", { value: m.value_ms })}
              </td>
              <td>{translateUi(sourceLabels[m.source])}</td>
            </tr>
          ))}
        </tbody>
      </table>
      {report.unavailable.length > 0 && (
        <p>
          {translateUi("Unavailable phases: {{phases}}", {
            phases: report.unavailable
              .map((phase) => translateUi(phaseLabels[phase]))
              .join(", "),
          })}
        </p>
      )}
      <p>
        {translateUi(
          "Client playback feedback does not measure physical hearing. Transcription turnaround and response generation can include multiple stages.",
        )}
      </p>
    </details>
  );
}
