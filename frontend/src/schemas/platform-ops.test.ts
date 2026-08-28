import { describe, expect, it } from "vitest";
import {
  adminPricing,
  discoveryPricing,
  perCallPricing,
} from "@/schemas/__fixtures__/platform-ops-builders";
import {
  callAndSayUpdateSchema,
  flightSearchUpdateSchema,
  platformOperationDiscoveryListSchema,
  platformOperationListSchema,
  speakUpdateSchema,
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
              max_calls_per_user_per_day: 50,
            },
            updated_at: null,
            updated_by: null,
            vendor_service_id: "platform-elevenlabs-id",
            pricing: adminPricing(),
          },
          {
            op: "call_and_say",
            enabled: false,
            vendor_service_slug: "platform-twilio",
            config: {
              type: "call_and_say",
              allowed_destination_prefixes: [],
              max_message_chars: 500,
              max_duration_seconds: 60,
              voice: "alice",
              max_calls_per_user_per_day: 3,
              account_sid: "",
              call_from: "",
            },
            updated_at: null,
            updated_by: null,
            vendor_service_id: "platform-twilio-id",
            pricing: perCallPricing("0.25"),
          },
        ],
      }).operations,
    ).toHaveLength(2);
  });

  it("enforces the server hard caps", () => {
    expect(
      speakUpdateSchema.safeParse({
        enabled: true,
        vendor_service_slug: "platform-elevenlabs",
        config: {
          type: "speak",
          allowed_voice_ids: ["voice-a"],
          max_chars: 5001,
          model_id: "eleven_multilingual_v2",
          max_calls_per_user_per_day: 50,
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
          max_duration_seconds: 60,
          voice: "alice",
          max_calls_per_user_per_day: 3,
          account_sid: `AC${"0".repeat(32)}`,
          call_from: "+6512345678",
        },
      }).success,
    ).toBe(false);
    expect(
      flightSearchUpdateSchema.safeParse({
        enabled: true,
        vendor_service_slug: "platform-duffel",
        config: {
          type: "flight_search",
          max_offers_cap: 51,
          max_searches_per_user_per_day: 20,
        },
      }).success,
    ).toBe(false);
  });

  it("parses discovery without accepting credential or account fields", () => {
    const result = platformOperationDiscoveryListSchema.parse({
      operations: [
        {
          op: "flight_search",
          display_name: "Flight Search",
          description: "Searches bounded flight offers.",
          vendor: "duffel",
          catalog_service_slug: "duffel",
          credential_source: "platform",
          credential_intent: "auto",
          availability_reason: null,
          fallback_reason: "own_credential_absent",
          own_connection: null,
          pricing: discoveryPricing({
            billable: true,
            price_per_unit: "0.5",
            display: "0.5 credits per call",
          }),
          mcp_tool: "nyx__flight_search",
        },
      ],
    });
    expect(result.operations[0]?.mcp_tool).toBe("nyx__flight_search");

    expect(
      platformOperationDiscoveryListSchema.safeParse({
        operations: [
          {
            ...result.operations[0],
            credential_source: "own_connection",
            fallback_reason: null,
            own_connection: {
              user_service_id: "duffel-key",
              slug: "duffel",
              label: "My Duffel",
              is_active: true,
              usable: false,
              reason: "approval_required",
            },
          },
        ],
      }).success,
    ).toBe(true);

    expect(
      platformOperationDiscoveryListSchema.safeParse({
        operations: [
          {
            ...result.operations[0],
            credential_source: "unavailable",
            credential_intent: "own_only",
            availability_reason: "own_connection_disabled",
            fallback_reason: null,
            own_connection: {
              user_service_id: "duffel-key",
              slug: "duffel",
              label: "My Duffel",
              is_active: false,
              usable: false,
              reason: "disabled",
            },
          },
        ],
      }).success,
    ).toBe(true);

    expect(
      platformOperationDiscoveryListSchema.safeParse({
        operations: [
          {
            ...result.operations[0],
            vendor_account_id: "should-not-be-exposed",
          },
        ],
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
        max_calls_per_user_per_day: 50,
      },
    });
    const call = callAndSayUpdateSchema.safeParse({
      enabled: false,
      vendor_service_slug: "platform-twilio",
      config: {
        type: "call_and_say",
        allowed_destination_prefixes: [],
        max_message_chars: 500,
        max_duration_seconds: 60,
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
        max_calls_per_user_per_day: 50,
      },
    });
    const malformedPrefix = callAndSayUpdateSchema.safeParse({
      enabled: true,
      vendor_service_slug: "platform-twilio",
      config: {
        type: "call_and_say",
        allowed_destination_prefixes: ["65"],
        max_message_chars: 500,
        max_duration_seconds: 60,
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
      speakUpdateSchema.safeParse({
        enabled: true,
        vendor_service_slug: "platform-elevenlabs",
        config: { type: "flight_search", max_results_cap: 10 },
      }).success,
    ).toBe(false);
    expect(
      speakUpdateSchema.safeParse({
        enabled: true,
        vendor_service_slug: "platform-elevenlabs",
        config: {
          type: "speak",
          allowed_voice_ids: ["voice-a"],
          max_chars: 1_000,
          model_id: "eleven_multilingual_v2",
          max_calls_per_user_per_day: 50,
        },
        vendor_body: { query: "caller controlled" },
      }).success,
    ).toBe(false);
  });
});
