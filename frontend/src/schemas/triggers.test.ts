import { describe, expect, it } from "vitest";
import {
  buildCreateTriggerRequest,
  createTriggerResponseSchema,
  listTriggersResponseSchema,
  triggerFormSchema,
  triggerResponseSchema,
} from "./triggers";

const triggerWire = {
  id: "80c7e6d9-41d5-48c3-bfd7-2bf9c92fa288",
  user_id: "1ca34962-7698-46a0-85d0-c85445becd72",
  label: "Repository activity",
  user_service_id: null,
  status: "active",
  verification: { mode: "token", location: "bearer" },
  delivery: { type: "notification" },
  inbound_url:
    "https://api.example.com/api/v1/webhooks/triggers/80c7e6d9-41d5-48c3-bfd7-2bf9c92fa288",
  created_at: "2026-08-06T09:30:00.123+00:00",
  updated_at: "2026-08-06T09:30:00.123+00:00",
} as const;

describe("trigger schemas", () => {
  it("parses list items and status enums from the real wire shape", () => {
    const parsed = listTriggersResponseSchema.parse({ triggers: [triggerWire] });
    expect(parsed.triggers[0]).toEqual(triggerWire);
    expect(triggerResponseSchema.safeParse({ ...triggerWire, status: "pending" }).success)
      .toBe(false);
  });

  it("parses create responses with both one-time secrets", () => {
    const parsed = createTriggerResponseSchema.parse({
      trigger: {
        ...triggerWire,
        verification: {
          mode: "hmac_sha256",
          header_name: "X-Hub-Signature-256",
        },
        delivery: {
          type: "webhook",
          url: "https://receiver.example.com/events",
        },
      },
      secret: "nyx_trg_once",
      delivery_signing_secret: "nyx_whsec_once",
    });

    expect(parsed.secret).toBe("nyx_trg_once");
    expect(parsed.delivery_signing_secret).toBe("nyx_whsec_once");
  });

  it("builds exact tagged unions for conditional create fields", () => {
    const values = triggerFormSchema.parse({
      label: "Repository activity",
      verification_mode: "hmac",
      signature_header: "X-Hub-Signature-256",
      delivery_type: "agent",
      webhook_url: "",
      conversation_id: "conversation-1",
    });

    expect(buildCreateTriggerRequest(values)).toEqual({
      label: "Repository activity",
      verification: {
        mode: "hmac_sha256",
        header_name: "X-Hub-Signature-256",
      },
      delivery: { type: "agent", conversation_id: "conversation-1" },
    });
  });

  it("requires only the fields selected by verification and delivery", () => {
    const missingHeader = triggerFormSchema.safeParse({
      label: "Repository activity",
      verification_mode: "hmac",
      signature_header: "",
      delivery_type: "notification",
      webhook_url: "",
      conversation_id: "",
    });
    const missingWebhook = triggerFormSchema.safeParse({
      label: "Repository activity",
      verification_mode: "bearer",
      signature_header: "",
      delivery_type: "webhook",
      webhook_url: "",
      conversation_id: "",
    });

    expect(missingHeader.success).toBe(false);
    expect(missingWebhook.success).toBe(false);
  });
});
