import { describe, expect, it } from "vitest";
import {
  callAndSayUpdateSchema,
  platformOperationListSchema,
  platformVendorProvisionSchema,
  platformVendorRequirementListSchema,
  platformVendorTemplateFormSchema,
  speakUpdateSchema,
  xSearchUpdateSchema,
} from "./platform-ops";

describe("platform operation schemas", () => {
  it("parses the four-vendor provisioning contract", () => {
    const result = platformVendorRequirementListSchema.parse({
      vendors: [
        {
          id: "template-elevenlabs",
          vendor: "elevenlabs",
          display_name: "ElevenLabs",
          operation: "speak",
          slug: "platform-elevenlabs",
          base_url: "https://api.elevenlabs.io",
          auth_method: "header",
          auth_key_name: "xi-api-key",
          service_category: "internal",
          visibility: "public",
          credential_label: "API key",
          credential_note: "Use a restricted key.",
          capability_summary: "Serves speak.",
          restriction_summary: "Does not expose vendor tools.",
          is_active: true,
          is_seeded: true,
          existing_service: null,
        },
        {
          id: "template-duffel",
          vendor: "duffel",
          display_name: "Duffel",
          operation: null,
          slug: "platform-duffel",
          base_url: "https://api.duffel.com",
          auth_method: "bearer",
          auth_key_name: null,
          service_category: "internal",
          visibility: "public",
          credential_label: "Access token",
          credential_note: "Provision ahead of an operation.",
          capability_summary: "No operation is shipped yet.",
          restriction_summary: "Does not expose vendor tools.",
          is_active: true,
          is_seeded: true,
          existing_service: null,
        },
      ],
    });

    expect(result.vendors[0]?.auth_key_name).toBe("xi-api-key");
    expect(result.vendors[1]?.operation).toBeNull();
  });

  it("requires a write-only credential and bounds the optional note", () => {
    expect(
      platformVendorProvisionSchema.safeParse({
        vendor: "x",
        credential: "",
        note: "",
      }).success,
    ).toBe(false);
    expect(
      platformVendorProvisionSchema.safeParse({
        vendor: "x",
        credential: "secret",
        note: "n".repeat(4097),
      }).success,
    ).toBe(false);
    expect(
      platformVendorProvisionSchema.safeParse({
        vendor: "x",
        credential: "secret",
        note: "Read-only app token",
      }).success,
    ).toBe(true);
  });

  it("accepts extensible vendor templates and rejects an invalid template shape", () => {
    const template = platformVendorTemplateFormSchema.safeParse({
      vendor: "acme_voice",
      display_name: "Acme Voice",
      slug: "platform-acme-voice",
      base_url: "https://api.acme.example",
      auth_method: "header",
      auth_key_name: "X-Acme-Key",
      credential_label: "API key",
      credential_note: "Use a restricted production key.",
      operation: null,
      capability_summary: "Provides the Acme voice operation.",
      restriction_summary: "Does not expose the vendor catalog.",
      is_active: true,
    });
    expect(template.success).toBe(true);

    expect(
      platformVendorTemplateFormSchema.safeParse({
        vendor: "Acme",
        display_name: "Acme Voice",
        slug: "platform-acme-voice",
        base_url: "https://api.acme.example",
        auth_method: "bearer",
        auth_key_name: null,
        credential_label: "Access token",
        credential_note: "note",
        operation: null,
        capability_summary: "capability",
        restriction_summary: "restriction",
        is_active: true,
      }).success,
    ).toBe(false);
  });

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
