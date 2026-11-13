import type { VoiceOption, VoicePreferences } from "./voiceContracts";

// Server capability and per-thread eligibility still gate every voice entry.
export const NATIVE_VOICE_ENTRY_ENABLED = true;

export function voiceOptionKey(option: VoiceOption | VoicePreferences) {
  return JSON.stringify([
    option.service_id,
    option.connection_id,
    option.key_source,
    option.model,
  ]);
}

/** Preserve a saved source; only first use chooses the web composer's default. */
export function chooseVoiceOption(
  options: readonly VoiceOption[],
  saved?: VoicePreferences | null,
) {
  if (saved) {
    const savedKey = voiceOptionKey(saved);
    return options.find((option) => voiceOptionKey(option) === savedKey);
  }
  return (
    options.find(
      (option) =>
        option.available &&
        option.default_model &&
        option.key_source === "platform",
    ) ??
    options.find((option) => option.available && option.default_model) ??
    options.find((option) => option.available)
  );
}

export function voiceTariff(option: VoiceOption) {
  if (option.key_source === "own" && !option.pricing) {
    const prices = option.reported_token_pricing;
    const tokens = prices
      ? [prices, ...prices.components].filter(
          (price) =>
            price.sync_status === "synced" && price.metric.includes("tokens"),
        )
      : [];
    return `No NyxID ${tokens.length ? "duration" : "voice"} charge. Your provider’s charges still apply.${tokens
      .map(
        (price) =>
          ` ${price.credits_per_unit} credits per ${price.metric.replaceAll("_", " ")}.`,
      )
      .join("")}`;
  }
  const prices = option.pricing
    ? [option.pricing, ...option.pricing.components]
    : [];
  const duration = prices.find(
    (price) =>
      price.metric === "voice_seconds" && price.sync_status === "synced",
  );
  if (!duration) return "Voice tariff unavailable.";
  const extras = prices.filter(
    (price) =>
      price.sync_status === "synced" &&
      price.metric !== "voice_seconds" &&
      (price.metric === "requests" ||
        (option.voice.protocol === "xai_realtime" &&
          price.metric.includes("tokens")) ||
        option.voice.billing_metrics.some((metric) => metric === price.metric)),
  );
  return `${duration.credits_per_unit} credits per second${extras
    .map(
      (price) =>
        ` plus ${price.credits_per_unit} per ${price.metric === "requests" ? "start" : price.metric.replaceAll("_", " ")}`,
    )
    .join("")}.`;
}

export function voiceElapsed(
  createdAt?: string,
  closedAt?: string | null,
  now = Date.now(),
) {
  const elapsed = createdAt
    ? Math.max(
        0,
        Math.floor(
          ((closedAt ? Date.parse(closedAt) : now) - Date.parse(createdAt)) /
            1000,
        ),
      )
    : 0;
  return `${Math.floor(elapsed / 60)}:${String(elapsed % 60).padStart(2, "0")}`;
}

/** Mirrors Web: private specialists and NyxBot both keep Voice when empty. */
export function composerTrailingControl(input: {
  enabled: boolean;
  running: boolean;
  conversationId?: string;
  text: string;
  attachmentCount: number;
  uploadBlocked: boolean;
  recording?: boolean;
  channel?: unknown;
  destroyed?: boolean;
  canCompose: boolean;
  canSend: boolean;
  voiceOpen?: boolean;
  openingVoice?: boolean;
  stopDisabled?: boolean;
}): { kind: "send" | "stop" | "voice"; disabled?: boolean; busy?: boolean } {
  if (input.running && input.conversationId) {
    return { kind: "stop", disabled: input.stopDisabled };
  }
  // Persisted `subagent` is the normal specialist role, not a voice restriction.
  if (
    input.enabled &&
    !input.running &&
    !input.text.trim() &&
    input.attachmentCount === 0 &&
    !input.uploadBlocked &&
    !input.recording &&
    !input.channel &&
    !input.destroyed &&
    !input.voiceOpen
  ) {
    return {
      kind: "voice",
      disabled: !input.canCompose || !!input.openingVoice,
      busy: input.openingVoice,
    };
  }
  return { kind: "send", disabled: !input.canSend };
}
