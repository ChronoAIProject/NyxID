import { z } from "zod";

export const adapterOperationSchema = z.enum([
  "receive_message",
  "send_message",
  "edit_message",
  "receive_call",
  "end_session",
  "receive_dtmf",
  "stream_reply",
  "interrupt",
  "dial",
  "transfer",
  "play_audio",
]);
export const adapterDescriptorSchema = z
  .object({
    adapter_id: z.string(),
    contract_version: z.number().int(),
    operations: z.array(z.string().min(1)),
    messaging: z
      .object({
        inbound_text: z.boolean(),
        outbound_text: z.boolean(),
        max_text_bytes: z.number().nullable(),
        delivery: z.string(),
      })
      .passthrough(),
    conversation: z
      .object({
        final_transcripts: z.boolean(),
        streamed_output: z.boolean(),
        interruption: z.boolean(),
        explicit_cancel: z.boolean(),
        playback_evidence: z.string(),
      })
      .passthrough(),
    instructions: z.object({
      external_nyxbot: z.boolean(),
      nyxbot_refresh: z.string(),
      provider_prompt_replacement: z.string(),
    }),
    lifecycle: z
      .object({
        max_session_seconds: z.number().nullable(),
        reconnect: z.boolean(),
        automatic_effect_replay: z.boolean(),
      })
      .passthrough(),
    setup: z
      .object({
        credential_modes: z.array(
          z.object({
            kind: z.string(),
            implemented: z.boolean(),
            prerequisite: z.string().nullable(),
          }),
        ),
        carrier: z.string(),
        speech: z.string(),
        agent: z.string(),
        documentation: z.url().nullable(),
      })
      .passthrough(),
  })
  .passthrough();
export const adapterCredentialSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("connection"), user_service_id: z.uuid() }),
  z.object({ kind: z.literal("platform"), catalog_service_id: z.uuid() }),
]);
export const adapterAvailabilitySchema = z
  .object({
    available: z.boolean(),
    reason: z.string().nullable(),
    operations: z.array(
      z.object({
        operation: z.string().min(1),
        implemented: z.boolean(),
        available: z.boolean(),
        reason: z.string().nullable(),
      }),
    ),
  })
  .passthrough();
export const adapterBindingSchema = z.object({
  id: z.uuid(),
  label: z.string(),
  adapter_id: z.string(),
  contract_version: z.number(),
  revision: z.number(),
  agent_id: z.string(),
  credential: adapterCredentialSchema,
  account_id: z.string(),
  resource_id: z.string(),
  phone_number: z.string(),
  application_id: z.string(),
  voice: z.string(),
  language: z.string(),
  enabled: z.boolean(),
  verified_at: z.string(),
  webhook_url: z.url(),
  availability: adapterAvailabilitySchema,
});
export const adapterSetupSchema = z
  .object({
    adapter_id: z.string().min(1),
    contract_version: z.literal(1),
    label: z.string().trim().min(1).max(80),
    agent_id: z.string().min(1),
    credential: adapterCredentialSchema,
    account_id: z.string().min(1).max(256),
    resource_id: z.string().min(1).max(256),
    phone_number: z.string(),
    application_id: z.string().min(1).max(256),
    webhook_public_key: z.string().nullable(),
    voice: z.string().max(128),
    language: z.string().min(1).max(32),
  })
  .superRefine((v, ctx) => {
    if (
      !v.adapter_id.startsWith("elevenlabs.") &&
      !/^\+[0-9]{7,15}$/.test(v.phone_number)
    )
      ctx.addIssue({
        code: "custom",
        path: ["phone_number"],
        message: "Use an international number, for example +14155550123.",
      });
    if (
      v.adapter_id.startsWith("telnyx.") &&
      v.credential.kind === "connection" &&
      !v.webhook_public_key
    )
      ctx.addIssue({
        code: "custom",
        path: ["webhook_public_key"],
        message:
          "Enter the account's Ed25519 public key from Telnyx Mission Control.",
      });
  });
export type AdapterDescriptor = z.infer<typeof adapterDescriptorSchema>;
export type AdapterBinding = z.infer<typeof adapterBindingSchema>;
export type AdapterSetup = z.infer<typeof adapterSetupSchema>;
export type AdapterOperation = z.infer<typeof adapterOperationSchema>;
export const adapterReason: Record<string, string> = {
  disabled:
    "Enable NyxBot and select an active specialist, then verify this binding.",
  connection_required: "Reconnect the selected service credential.",
  verification_required:
    "The account or credential changed. Verify the binding again.",
  scope_missing:
    "The connection's operation policy does not permit a required provider operation.",
  resource_policy_unresolved:
    "Call-specific policy is evaluated when the provider call ID is known.",
  billing_unavailable:
    "An administrator must configure and synchronize carrier pricing and the resource tariff.",
  resource_not_allocated:
    "An administrator must allocate this number and application to your account.",
  unsupported_operation: "This profile does not implement this operation.",
  provider_limit_unavailable:
    "Telnyx inbound Answer/relay cannot enforce a call-leg time cap. Platform calling is unavailable; use your own Telnyx connection.",
  external_speech_billing_required:
    "This Custom LLM profile uses your ElevenLabs account for speech. Select your own connection.",
};

export const adapterAccountSchema = z.object({
  id: z.uuid(),
  catalog_service_id: z.uuid(),
  provider: z.string(),
  account_id: z.string(),
  webhook_key_configured: z.boolean(),
  revision: z.number(),
  configured_by: z.string(),
  updated_at: z.string(),
});
export const adapterAllocationSchema = z.object({
  id: z.uuid(),
  catalog_service_id: z.uuid(),
  owner_id: z.uuid(),
  provider: z.string(),
  account_id: z.string(),
  resource_id: z.string(),
  phone_number: z.string(),
  application_id: z.string(),
  revision: z.number(),
  tariff_version: z.string(),
  tariff_source: z.url(),
  includes_all_provider_charges: z.boolean(),
  tariff_verified_at: z.string(),
});
export const adapterAccountInputSchema = z
  .object({
    catalog_service_id: z.uuid(),
    adapter_id: z.enum(["telnyx.messaging", "twilio.messaging"]),
    account_id: z.string().min(1).max(256),
    webhook_public_key: z.string().nullable(),
  })
  .superRefine((v, ctx) => {
    if (v.adapter_id.startsWith("telnyx") && !v.webhook_public_key)
      ctx.addIssue({
        code: "custom",
        path: ["webhook_public_key"],
        message: "Enter this platform account's Ed25519 public key.",
      });
  });
export const adapterAllocationInputSchema = z.object({
  setup: z.object({
    adapter_id: z.enum([
      "telnyx.messaging",
      "twilio.messaging",
      "twilio.conversation_relay",
    ]),
    credential: z.object({
      kind: z.literal("platform"),
      catalog_service_id: z.uuid(),
    }),
    account_id: z.string().min(1),
    resource_id: z.string().min(1).max(256),
    phone_number: z.string().regex(/^\+[0-9]{7,15}$/),
    application_id: z.string().min(1).max(256),
  }),
  owner_id: z.uuid(),
  tariff_version: z.string().min(1).max(128),
  tariff_source: z.url().startsWith("https://"),
  includes_all_provider_charges: z.boolean().refine((v) => v, {
    message: "Confirm that the synchronized price covers all provider charges.",
  }),
});
export type AdapterAccountInput = z.infer<typeof adapterAccountInputSchema>;
export type AdapterAllocationInput = z.infer<
  typeof adapterAllocationInputSchema
>;
