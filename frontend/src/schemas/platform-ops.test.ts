import { describe, expect, it } from "vitest";
import {
  callAndSayUpdateSchema,
  platformOperationListSchema,
  speakUpdateSchema,
  xSearchUpdateSchema,
} from "./platform-ops";

describe("platform operation schemas", () => {
  it("accepts the disabled defaults returned for missing rows", () => {
    expect(
      platformOperationListSchema.parse({
        operations: [
          {
            op: "speak",
            enabled: false,
            vendor_service_slug: "platform-elevenlabs",
            config: {
              type: "speak",
              allowed_voice_ids: [],
              max_chars: 1000,
              model_id: "eleven_multilingual_v2",
            },
            updated_at: null,
            updated_by: null,
          },
          {
            op: "call_and_say",
            enabled: false,
            vendor_service_slug: "platform-twilio",
            config: {
              type: "call_and_say",
              allowed_destination_prefixes: [],
              max_message_chars: 500,
              voice: "alice",
              max_calls_per_user_per_day: 3,
              account_sid: "",
              call_from: "",
            },
            updated_at: null,
            updated_by: null,
          },
        ],
      }).operations,
    ).toHaveLength(2);
  });

  it("enforces the server hard caps", () => {
    expect(
      xSearchUpdateSchema.safeParse({
        enabled: true,
        vendor_service_slug: "platform-x",
        config: { type: "x_search", max_results_cap: 26 },
      }).success,
    ).toBe(false);
    expect(
      speakUpdateSchema.safeParse({
        enabled: true,
        vendor_service_slug: "platform-elevenlabs",
        config: {
          type: "speak",
          allowed_voice_ids: ["voice-a"],
          max_chars: 5001,
          model_id: "eleven_multilingual_v2",
        },
      }).success,
    ).toBe(false);
    expect(
      callAndSayUpdateSchema.safeParse({
        enabled: true,
        vendor_service_slug: "platform-twilio",
        config: {
          type: "call_and_say",
          allowed_destination_prefixes: ["+65"],
          max_message_chars: 1001,
          voice: "alice",
          max_calls_per_user_per_day: 3,
          account_sid: `AC${"0".repeat(32)}`,
          call_from: "+6512345678",
        },
      }).success,
    ).toBe(false);
  });

  it("requires a voice allowlist but permits an empty destination allowlist", () => {
    const speak = speakUpdateSchema.safeParse({
      enabled: false,
      vendor_service_slug: "platform-elevenlabs",
      config: {
        type: "speak",
        allowed_voice_ids: [],
        max_chars: 1000,
        model_id: "eleven_multilingual_v2",
      },
    });
    const call = callAndSayUpdateSchema.safeParse({
      enabled: false,
      vendor_service_slug: "platform-twilio",
      config: {
        type: "call_and_say",
        allowed_destination_prefixes: [],
        max_message_chars: 500,
        voice: "alice",
        max_calls_per_user_per_day: 3,
        account_sid: `AC${"a".repeat(32)}`,
        call_from: "+6512345678",
      },
    });

    expect(speak.success).toBe(false);
    expect(call.success).toBe(true);
  });

  it("rejects duplicate or malformed allowlist values", () => {
    const duplicateVoices = speakUpdateSchema.safeParse({
      enabled: true,
      vendor_service_slug: "platform-elevenlabs",
      config: {
        type: "speak",
        allowed_voice_ids: ["voice-a", "voice-a"],
        max_chars: 1000,
        model_id: "eleven_multilingual_v2",
      },
    });
    const malformedPrefix = callAndSayUpdateSchema.safeParse({
      enabled: true,
      vendor_service_slug: "platform-twilio",
      config: {
        type: "call_and_say",
        allowed_destination_prefixes: ["65"],
        max_message_chars: 500,
        voice: "alice",
        max_calls_per_user_per_day: 3,
        account_sid: `AC${"b".repeat(32)}`,
        call_from: "+6512345678",
      },
    });

    expect(duplicateVoices.success).toBe(false);
    expect(malformedPrefix.success).toBe(false);
  });

  it("rejects unknown fields and mismatched config types", () => {
    expect(
      xSearchUpdateSchema.safeParse({
        enabled: true,
        vendor_service_slug: "platform-x",
        config: { type: "speak", max_results_cap: 10 },
      }).success,
    ).toBe(false);
    expect(
      xSearchUpdateSchema.safeParse({
        enabled: true,
        vendor_service_slug: "platform-x",
        config: { type: "x_search", max_results_cap: 10 },
        vendor_body: { query: "caller controlled" },
      }).success,
    ).toBe(false);
  });
});
