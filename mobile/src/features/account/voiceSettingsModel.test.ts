import assert from "node:assert/strict";
import test from "node:test";
import { voiceSettingsView } from "./voiceSettingsModel";

const option = (overrides: Record<string, unknown> = {}) =>
  ({
    service_id: "11111111-1111-4111-8111-111111111111",
    connection_id: null,
    key_source: "platform",
    model: "gpt-live",
    model_label: "Live",
    default_model: true,
    available: true,
    unavailable_reason: null,
    billing_owner: "me",
    pricing: null,
    voice: {
      protocol: "openai_live",
      models: [{ id: "gpt-live", label: "Live" }],
      voices: [
        { id: "alloy", label: "Alloy" },
        { id: "verse", label: "Verse" },
      ],
      usage_source: "provider_reported",
      billing_metrics: ["voice_seconds"],
    },
    ...overrides,
  }) as any;

test("Settings uses the call's automatic source with the saved voice, mode and notice", () => {
  const view = voiceSettingsView([option()], {
    service_id: "11111111-1111-4111-8111-111111111111",
    connection_id: null,
    key_source: "platform",
    model: "gpt-live",
    voice: "verse",
    input_mode: "automatic",
    language: null,
    notify_on_completion: true,
  })!;
  assert.equal(view.preferences.voice, "verse");
  assert.equal(view.preferences.input_mode, "automatic");
  assert.equal(view.preferences.notify_on_completion, true);
  assert.equal(view.holdOnly, false);
});

test("first use defaults to the first voice, Hold to talk and no notice", () => {
  const view = voiceSettingsView([option()], null)!;
  assert.deepEqual(
    [view.preferences.voice, view.preferences.input_mode, view.preferences.notify_on_completion, view.preferences.language],
    ["alloy", "push_to_talk", false, null],
  );
});

test("a saved voice the source no longer offers falls back, and Grok is always Hold to talk", () => {
  const grok = option({
    voice: { ...option().voice, protocol: "xai_realtime", usage_source: "server_measured" },
  });
  const view = voiceSettingsView([grok], {
    ...voiceSettingsView([option()], null)!.preferences,
    voice: "gone",
    input_mode: "automatic",
  })!;
  assert.equal(view.preferences.voice, "alloy");
  assert.equal(view.preferences.input_mode, "push_to_talk");
  assert.equal(view.holdOnly, true);
});

test("no available source means nothing to configure", () => {
  assert.equal(voiceSettingsView([option({ available: false })], null), null);
  assert.equal(voiceSettingsView([], null), null);
});
