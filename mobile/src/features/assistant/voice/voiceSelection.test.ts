import assert from "node:assert/strict";
import test from "node:test";
import type { VoiceOption, VoicePreferences } from "./voiceContracts";
import {
  chooseVoiceOption,
  showVoiceComposer,
  voiceElapsed,
  voiceOptionKey,
  voiceTariff,
} from "./voiceSelection";

const serviceId = "11111111-1111-4111-8111-111111111111";
const connectionId = "22222222-2222-4222-8222-222222222222";
function option(patch: Partial<VoiceOption> = {}): VoiceOption {
  return {
    service_id: serviceId,
    connection_id: null,
    key_source: "platform",
    model: "gpt-live-1",
    model_label: "GPT Live",
    default_model: true,
    voice: {
      protocol: "openai_live",
      models: [{ id: "gpt-live-1", label: "GPT Live", default: true }],
      voices: [{ id: "marin", label: "Marin" }],
      usage_source: "provider_reported",
      billing_metrics: ["voice_seconds"],
    },
    available: true,
    unavailable_reason: null,
    billing_owner: "person",
    pricing: null,
    ...patch,
  };
}
function preferences(value: VoiceOption): VoicePreferences {
  return {
    service_id: value.service_id,
    connection_id: value.connection_id,
    key_source: value.key_source,
    model: value.model,
    voice: "marin",
    input_mode: "push_to_talk",
    language: null,
    notify_on_completion: false,
  };
}

test("automatic voice selection matches web: available default, then first available, then none", () => {
  const disabled = option({ model: "disabled", available: false });
  const first = option({ model: "first", default_model: false });
  const preferred = option({ model: "default" });
  assert.equal(chooseVoiceOption([disabled, first, preferred]), preferred);
  assert.equal(chooseVoiceOption([disabled, first]), first);
  assert.equal(chooseVoiceOption([disabled]), undefined);
  assert.equal(chooseVoiceOption([]), undefined);
  assert.notEqual(voiceOptionKey(first), voiceOptionKey(preferred));
  assert.equal(voiceOptionKey(first), voiceOptionKey(preferences(first)));
});

test("platform tariffs retain exact decimal rates and only list synced applicable components", () => {
  const text = voiceTariff(
    option({
      pricing: {
        metric: "voice_seconds",
        credits_per_unit: "0.000000000001",
        sync_status: "synced",
        components: [
          {
            metric: "requests",
            credits_per_unit: "0.000000000013",
            sync_status: "synced",
          },
          {
            metric: "tokens",
            credits_per_unit: "0.123456789123",
            sync_status: "synced",
          },
          {
            metric: "voice_seconds",
            credits_per_unit: "99",
            sync_status: "pending",
          },
        ],
      },
    }),
  );
  assert.match(text, /0\.000000000001 credits per second/);
  assert.match(text, /0\.000000000013 per start/);
  assert.doesNotMatch(text, /0\.123456789123|99/);
  assert.doesNotMatch(text, /e-1[0-9]/);
});

test("xAI tariffs include reported token prices even when the duration metric list omits them", () => {
  const source = option();
  const text = voiceTariff(
    option({
      voice: {
        ...source.voice,
        protocol: "xai_realtime",
        usage_source: "server_measured",
      },
      pricing: {
        metric: "voice_seconds",
        credits_per_unit: "0.02",
        sync_status: "synced",
        components: [
          {
            metric: "input_tokens",
            credits_per_unit: "0.000000000021",
            sync_status: "synced",
          },
          {
            metric: "output_tokens",
            credits_per_unit: "0.000000000089",
            sync_status: "synced",
          },
          {
            metric: "cache_read_tokens",
            credits_per_unit: "0.9",
            sync_status: "failed",
          },
        ],
      },
    }),
  );
  assert.match(text, /0\.000000000021 per input tokens/);
  assert.match(text, /0\.000000000089 per output tokens/);
  assert.doesNotMatch(text, /0\.9|cache read/);
});

test("declared OpenAI billing metrics show only their synced prices", () => {
  const source = option();
  const text = voiceTariff(
    option({
      voice: {
        ...source.voice,
        billing_metrics: ["voice_seconds", "input_tokens"],
      },
      pricing: {
        metric: "voice_seconds",
        credits_per_unit: "0.02",
        sync_status: "synced",
        components: [
          {
            metric: "input_tokens",
            credits_per_unit: "0.001",
            sync_status: "synced",
          },
          {
            metric: "output_tokens",
            credits_per_unit: "0.009",
            sync_status: "synced",
          },
        ],
      },
    }),
  );
  assert.match(text, /0\.001 per input tokens/);
  assert.doesNotMatch(text, /0\.009|output tokens/);
});

test("own-key tariffs disclose provider charges and distinguish no duration from no voice charge", () => {
  const own = option({ key_source: "own", connection_id: connectionId });
  assert.match(voiceTariff(own), /No NyxID voice charge/);
  assert.match(voiceTariff(own), /provider’s charges still apply/);
  const text = voiceTariff({
    ...own,
    reported_token_pricing: {
      metric: "input_tokens",
      credits_per_unit: "0.000000000001",
      sync_status: "synced",
      components: [
        {
          metric: "output_tokens",
          credits_per_unit: "0.000000000002",
          sync_status: "synced",
        },
        { metric: "requests", credits_per_unit: "7", sync_status: "synced" },
        { metric: "tokens", credits_per_unit: "9", sync_status: "pending" },
      ],
    },
  });
  assert.match(text, /No NyxID duration charge/);
  assert.match(text, /0\.000000000001 credits per input tokens/);
  assert.match(text, /0\.000000000002 credits per output tokens/);
  assert.doesNotMatch(text, /7 credits|9 credits|requests/);
});

test("missing or unsynced platform duration prices never appear free", () => {
  assert.equal(voiceTariff(option()), "Voice tariff unavailable.");
  assert.equal(
    voiceTariff(
      option({
        pricing: {
          metric: "voice_seconds",
          credits_per_unit: "0.02",
          sync_status: "pending",
          components: [],
        },
      }),
    ),
    "Voice tariff unavailable.",
  );
});

test("composer offers voice only for an eligible empty private thread and never masks Send or Stop", () => {
  const ready = {
    enabled: true,
    running: false,
    text: "",
    attachmentCount: 0,
    uploadBlocked: false,
  };
  assert.equal(showVoiceComposer(ready), true);
  assert.equal(showVoiceComposer({ ...ready, text: "  \n\t" }), true);
  for (const patch of [
    { enabled: false },
    { running: true },
    { text: "你好" },
    { attachmentCount: 1 },
    { uploadBlocked: true },
    { channel: { platform: "telegram" } },
    { subagent: true },
    { destroyed: true },
  ])
    assert.equal(showVoiceComposer({ ...ready, ...patch }), false);
});

test("voice timer floors seconds, clamps future starts and freezes at the server close time", () => {
  const start = "2026-10-16T04:00:00Z";
  const now = Date.parse(start);
  assert.equal(voiceElapsed(undefined, null, now), "0:00");
  assert.equal(voiceElapsed(start, null, now - 1000), "0:00");
  assert.equal(voiceElapsed(start, null, now + 1999), "0:01");
  assert.equal(voiceElapsed(start, null, now + 61_000), "1:01");
  assert.equal(
    voiceElapsed(start, "2026-10-16T04:01:05Z", now + 999_000),
    "1:05",
  );
  assert.equal(voiceElapsed(start, null, now + 1800_000), "30:00");
});
