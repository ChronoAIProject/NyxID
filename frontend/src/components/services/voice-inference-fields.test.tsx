import { useState } from "react";
import { fireEvent, render, screen } from "@testing-library/react";
import { expect, it } from "vitest";
import { VoiceInferenceFields } from "./voice-inference-fields";
import {
  voiceMetadataSchema,
  type VoiceMetadata,
} from "@/schemas/platform-keys";
const metadata: VoiceMetadata = {
  protocol: "openai_live",
  usage_source: "provider_reported",
  models: [
    { id: "model-a", label: "A", default: true },
    { id: "model-b", label: "B" },
  ],
  voices: [{ id: "voice-a", label: "Voice A" }],
  billing_metrics: ["voice_seconds"],
};
it("edits catalog choices and moves the single default without seeding prices", () => {
  function Editor() {
    const [value, setValue] = useState<VoiceMetadata | null>(metadata);
    return (
      <>
        <VoiceInferenceFields value={value} onChange={setValue} />
        <output data-testid="value">{JSON.stringify(value)}</output>
      </>
    );
  }
  render(<Editor />);
  fireEvent.change(screen.getByLabelText("models 2 ID"), {
    target: { value: "catalog-new-model" },
  });
  fireEvent.click(screen.getByLabelText("Default model 2"));
  const saved = voiceMetadataSchema.parse(
    JSON.parse(screen.getByTestId("value").textContent!),
  );
  expect(saved.models).toEqual([
    { id: "model-a", label: "A", default: false },
    { id: "catalog-new-model", label: "B", default: true },
  ]);
  fireEvent.change(screen.getByLabelText("Voice protocol"), {
    target: { value: "none" },
  });
  expect(screen.getByTestId("value")).toHaveTextContent("null");
});
it("rejects duplicate choices, multiple defaults, endpoints and incompatible usage sources", () => {
  for (const invalid of [
    { ...metadata, models: [metadata.models[0], metadata.models[0]] },
    {
      ...metadata,
      models: metadata.models.map((m) => ({ ...m, default: true })),
    },
    { ...metadata, voices: [{ id: "https://evil.example", label: "URL" }] },
    { ...metadata, usage_source: "server_measured" },
    { ...metadata, billing_metrics: ["tokens"] },
  ])
    expect(voiceMetadataSchema.safeParse(invalid).success).toBe(false);
});
