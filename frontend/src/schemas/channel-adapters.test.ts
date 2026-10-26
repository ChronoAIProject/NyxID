import { describe, expect, it } from "vitest";
import { z } from "zod";
import catalogFixture from "./fixtures/channel-adapter-catalog.json";
import {
  adapterDescriptorSchema,
  adapterSetupSchema,
} from "./channel-adapters";

describe("executable adapter catalog contract", () => {
  function catalog() {
    return structuredClone(catalogFixture);
  }

  it("parses every registered adapter, including disabled and legacy profiles", () => {
    const value = z
      .object({ adapters: z.array(adapterDescriptorSchema) })
      .parse(catalog());
    const openclaw = value.adapters.find((a) => a.adapter_id === "openclaw");
    expect(openclaw).toMatchObject({
      setup: { documentation: null },
      messaging: { outbound_text: false },
    });
    expect(value.adapters.map((a) => a.adapter_id)).toEqual(
      expect.arrayContaining([
        "telnyx.conversation_relay",
        "telnyx.messaging",
        "twilio.conversation_relay",
        "twilio.messaging",
        "elevenlabs.custom_llm",
      ]),
    );
  });

  it("accepts additive operation values without dropping the catalog", () => {
    const value = catalog();
    const first = value.adapters[0];
    if (!first) throw new Error("Registered adapter catalog must not be empty");
    const descriptor = {
      ...first,
      operations: [...first.operations, "future_operation"],
      future_capability: { enabled: true },
    };
    expect(adapterDescriptorSchema.parse(descriptor).operations).toContain(
      "future_operation",
    );
  });

  it("requires valid credentials and the Telnyx public key for a user-owned connection", () => {
    const input = {
      adapter_id: "telnyx.messaging",
      contract_version: 1,
      label: "FAQ",
      agent_id: "specialist",
      credential: {
        kind: "connection",
        user_service_id: "11111111-1111-4111-8111-111111111111",
      },
      account_id: "account",
      resource_id: "number",
      application_id: "application",
      phone_number: "+14155550123",
      webhook_public_key: "public-key",
      voice: "Telnyx.Ultra.Callie",
      language: "en-US",
    };
    expect(adapterSetupSchema.safeParse(input).success).toBe(true);
    for (const change of [
      { phone_number: "5550123" },
      { webhook_public_key: null },
      { contract_version: 2 },
      { credential: { kind: "connection", user_service_id: "" } },
    ]) {
      expect(
        adapterSetupSchema.safeParse({ ...input, ...change }).success,
      ).toBe(false);
    }
    expect(
      adapterSetupSchema.safeParse({
        ...input,
        webhook_public_key: null,
        credential: {
          kind: "platform",
          catalog_service_id: "22222222-2222-4222-8222-222222222222",
        },
      }).success,
    ).toBe(true);
  });
});
