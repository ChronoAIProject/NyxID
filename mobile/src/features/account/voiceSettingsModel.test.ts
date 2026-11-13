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

test("Settings preserves the saved source, voice, mode and notice ahead of another default", () => {
  const savedSource = option({
    connection_id: "22222222-2222-4222-8222-222222222222",
    key_source: "own",
    model: "saved-model",
    default_model: false,
  });
  const view = voiceSettingsView([option(), savedSource], {
    service_id: "11111111-1111-4111-8111-111111111111",
    connection_id: savedSource.connection_id,
    key_source: "own",
    model: "saved-model",
    voice: "verse",
    input_mode: "automatic",
    language: null,
    notify_on_completion: true,
  })!;
  assert.equal(view.option, savedSource);
  assert.equal(view.preferences.connection_id, savedSource.connection_id);
  assert.equal(view.preferences.key_source, "own");
  assert.equal(view.preferences.model, "saved-model");
  assert.equal(view.preferences.voice, "verse");
  assert.equal(view.missingVoice, null);
  assert.equal(view.preferences.input_mode, "automatic");
  assert.equal(view.preferences.notify_on_completion, true);
  assert.equal(view.holdOnly, false);
});

test("first use matches iPad calls: first voice, hands-free and no notice", () => {
  const view = voiceSettingsView([option()], null)!;
  assert.deepEqual(
    [
      view.preferences.voice,
      view.preferences.input_mode,
      view.preferences.notify_on_completion,
      view.preferences.language,
    ],
    ["alloy", "automatic", false, null],
  );
  assert.equal(view.missingVoice, null);
});

test("a removed saved voice stays visible for explicit replacement, and Grok is always Hold to talk", () => {
  const grok = option({
    voice: {
      ...option().voice,
      protocol: "xai_realtime",
      usage_source: "server_measured",
    },
  });
  const view = voiceSettingsView([grok], {
    ...voiceSettingsView([option()], null)!.preferences,
    voice: "gone",
    input_mode: "automatic",
  })!;
  assert.equal(view.preferences.voice, "gone");
  assert.equal(view.missingVoice, "gone");
  assert.equal(view.preferences.input_mode, "push_to_talk");
  assert.equal(view.holdOnly, true);
});

test("no available source means nothing to configure", () => {
  assert.equal(voiceSettingsView([option({ available: false })], null), null);
  assert.equal(voiceSettingsView([], null), null);
});

test("Settings cannot rewrite a missing or unavailable saved source to an available default", () => {
  const savedSource = option({ model: "saved-model", available: false });
  const saved = {
    ...voiceSettingsView([option()], null)!.preferences,
    model: savedSource.model,
  };
  assert.equal(voiceSettingsView([option(), savedSource], saved), null);
  assert.equal(voiceSettingsView([option()], saved), null);
});

test("first-use Settings prefers an available platform default over an earlier own-key default", () => {
  const platform = option();
  const own = option({
    key_source: "own",
    connection_id: "22222222-2222-4222-8222-222222222222",
  });
  assert.equal(voiceSettingsView([own, platform], null)!.option, platform);
});
