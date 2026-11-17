import { z } from "zod";
import { parseCredits } from "@/lib/credits";
import {
  managedEligibilitySchema,
  managedVoiceOptionSchema,
} from "@/schemas/managed-lines";

// Zod keeps running refinements after a failed one, so this check parses on its own.
const positiveCredits = z.string().refine((value) => {
  try {
    return parseCredits(value) > 0n;
  } catch {
    return false;
  }
}, "Enter a positive credit amount");
const text = (max = 512) =>
  z
    .string()
    .trim()
    .min(1, "Required")
    .max(max, "Too long")
    .refine((v) => !/\p{Cc}/u.test(v), "Remove control characters");
const optionalText = (max = 512) =>
  z
    .string()
    .trim()
    .max(max, "Too long")
    .refine((v) => !/\p{Cc}/u.test(v), "Remove control characters");

export const managedCapabilitySchema = z.object({
  application_id: z.string(),
  billing_service_id: z.string(),
});
export const managedTariffSchema = z.object({
  kind: z.string(),
  amount: z.string(),
  currency: z.string(),
  unit: z.string(),
  increment: z.number().int(),
  minimum: z.number().int(),
  source: z.string(),
  revision: z.number().int(),
  fx_policy_revision: z.string(),
  effective_at: z.string(),
});
export const adminManagedClaimSchema = z.object({
  id: z.uuid(),
  owner_id: z.string(),
  generation: z.number().int(),
  state: z.string(),
  suspension: z.string().nullable(),
  voice_tier: z.string(),
  sms_enabled: z.boolean(),
  caller_confirmations: z.boolean(),
  auto_renew: z.boolean(),
  covered_until: z.string().nullable(),
});
export const adminManagedLineSchema = z.object({
  id: z.uuid(),
  provider: z.string(),
  trusted_account_id: z.string(),
  phone_number: z.string(),
  country: z.string(),
  number_resource_id: z.string(),
  voice: managedCapabilitySchema.nullable(),
  sms: managedCapabilitySchema.nullable(),
  rental_billing_service_id: z.string(),
  standard_voice_options: z.array(managedVoiceOptionSchema),
  regulatory: z.object({
    requirement_reference: z.string(),
    registration_status: z.string(),
    sms_sender_registration: z.string().nullable(),
    verified_by: z.string(),
    verified_at: z.string(),
  }),
  supplier_tariffs: z.array(managedTariffSchema),
  provider_channel_limit: z.number().int().nullable(),
  call_slot_limit: z.number().int(),
  calls_per_minute: z.number().int(),
  sms_per_minute: z.number().int(),
  state: z.string(),
  suspension: z.string().nullable(),
  claim: adminManagedClaimSchema.nullable(),
  live_calls: z.number().int(),
  cost_policy_revision: z.number().int().nullable(),
  voice_url: z.string(),
  sms_url: z.string(),
  revision: z.number().int(),
  available_at: z.string().nullable(),
  supplier_released_at: z.string().nullable(),
  created_at: z.string(),
  updated_at: z.string(),
});
export type AdminManagedLine = z.infer<typeof adminManagedLineSchema>;
/** Stored rows the server skipped because they could not be read (older servers omit it). */
const unreadableIds = z.array(z.string()).optional();
export const adminManagedLinePageSchema = z.object({
  lines: z.array(adminManagedLineSchema),
  next_after_id: z.string().nullish(),
  premium_enabled: z.boolean(),
  unreadable_ids: unreadableIds,
});

export const managedUnderwritingSchema = z.object({
  revision: z.number().int(),
  enabled: z.boolean(),
  accepted_by: z.string(),
  accepted_at: z.string(),
  acceptance_text: z.string(),
  acceptance_version: z.string(),
  paid_tier_evidence_reference: z.string(),
  max_leg_seconds: z.number().int(),
  max_leg_evidence_reference: z.string(),
  timestamp_fixture_reference: z.string().nullable(),
  detail_record_options_reference: z.string().nullable(),
  detail_record_fixture_reference: z.string().nullable(),
  detail_record_fields: z.array(z.string()),
  supplier_tariff_revision: z.number().int(),
  fx_policy_revision: z.string(),
  character_tariff_reference: z.string(),
  components: z.array(
    z.object({
      kind: z.string(),
      credits_per_second: z.string(),
      increment_seconds: z.number().int(),
      minimum_seconds: z.number().int(),
    }),
  ),
  rolling_budget: z.string(),
});
export const managedVoiceAccountSchema = z.object({
  revision: z.number().int(),
  per_call_characters: z.number().int(),
  greeting: z.string().nullable(),
  wrapup: z.string().nullable(),
  premium: z
    .object({
      api_key_ref: z.string(),
      voices: z.array(managedVoiceOptionSchema),
      billing_account_identity: z.string(),
      monthly_characters: z.number().int(),
      plan_tier: z.string(),
      oem_terms_version: z.string(),
      acknowledged_by: z.string(),
      acknowledged_at: z.string(),
      billing_service_id: z.string(),
    })
    .nullable(),
});
export const managedAccountSchema = z.object({
  id: z.uuid(),
  provider: z.string(),
  account_id: z.string(),
  catalog_service_id: z.string(),
  revision: z.number().int(),
  underwriting: managedUnderwritingSchema.nullable(),
  voice: managedVoiceAccountSchema.nullable(),
});
export type ManagedAccount = z.infer<typeof managedAccountSchema>;
export const managedAccountPageSchema = z.object({
  accounts: z.array(managedAccountSchema),
  next_after_id: z.string().nullish(),
  premium_enabled: z.boolean(),
  unreadable_ids: unreadableIds,
});

export const managedEligibilityPageSchema = z.object({
  eligibility: z.array(managedEligibilitySchema),
  next_after_id: z.string().nullish(),
});

export const managedLineOperationSchema = z.object({
  line_id: z.uuid(),
  claim_id: z.uuid().nullable(),
  revision: z.number().int(),
  state: z.string(),
  suspension: z.string().nullable(),
  live_calls: z.number().int(),
  termination_pending: z.boolean(),
});
export type ManagedLineOperation = z.infer<typeof managedLineOperationSchema>;
export const managedLinePreparationSchema = z.object({
  line_id: z.uuid(),
  voice_url: z.string(),
  sms_url: z.string(),
});
export type ManagedLinePreparation = z.infer<typeof managedLinePreparationSchema>;

export const managedReportDaySchema = z.object({
  day: z.string(),
  billed_credits: z.string().nullable(),
  provider_actual_cost_credits: z.string(),
  estimated_cost_credits: z.string(),
  inventory_cost_credits: z.string().nullable(),
  estimated_margin_credits: z.string().nullable(),
  actual_margin_credits: z.string().nullable(),
  unknown_components: z.number().int(),
  absorbed_seconds: z.number().int(),
  absorbed_sms_parts: z.number().int(),
  effect_count: z.number().int(),
});
export const managedReportSchema = z.object({
  lines: z.array(
    z.object({
      line_id: z.string(),
      days: z.array(managedReportDaySchema),
      budgets: z.array(
        z.object({
          kind: z.string(),
          limit: z.string(),
          recognized: z.string(),
          unresolved_holds: z.string(),
        }),
      ),
      inventory_policy_missing: z.boolean(),
    }),
  ),
  next_after: z.string().nullable(),
});
export type ManagedReport = z.infer<typeof managedReportSchema>;
export const managedEffectPageSchema = z.object({
  effects: z.array(
    z.object({
      id: z.string(),
      effect_id: z.string(),
      kind: z.string().optional(),
      created_at: z.string().optional(),
      component_coverage: z.boolean(),
      evidence_quality: z.string(),
      original_costs: z.array(
        z.object({
          kind: z.string(),
          source: z.string(),
          amount: z.string().nullable(),
          currency: z.string(),
          fx_policy_revision: z.string(),
          supplier_rounded_quantity: z.number().int().nullable(),
        }),
      ),
    }),
  ),
  next_after: z.string().nullable(),
});
export type ManagedEffectPage = z.infer<typeof managedEffectPageSchema>;
export const managedIntegritySchema = z.object({
  report_path: z.string(),
  pending_cost_effects: z.number().int(),
  unknown_inventory_lines: z.number().int(),
});

/** JSON editors are validated before submission and never contain secrets. */
const jsonArray = <T extends z.ZodType>(item: T, message: string) =>
  z.string().refine((raw) => {
    try {
      return z.array(item).safeParse(JSON.parse(raw)).success;
    } catch {
      return false;
    }
  }, message);
const tariffInputSchema = z.object({
  kind: z.enum(["carrier_inbound", "relay", "stt", "tts", "premium_tts", "sms_part"]),
  amount: z.string(),
  currency: z.string().regex(/^[A-Z]{3}$/),
  unit: z.string().min(1),
  increment: z.number().int().positive(),
  minimum: z.number().int().nonnegative(),
  source: z.string().startsWith("https://"),
  revision: z.number().int().positive(),
  fx_policy_revision: z.string().min(1),
  effective_at: z.string().min(1),
});
const voiceOptionsJson = jsonArray(
  managedVoiceOptionSchema,
  "Enter a JSON list of {voice, language} pairs",
);

export const eligibilityFormSchema = z
  .object({
    owner_id: z.uuid("Choose a person"),
    business_legal_name: text(),
    registration_id: text(),
    jurisdiction: z.string().regex(/^[A-Z]{2}$/, "Use an ISO country code such as SG"),
    allowed_use_case: text(),
    sms_sender_path: text(),
    sms_registration_status: text(),
    managed_line_terms: text(128),
    elevenlabs_eua: optionalText(128),
    premium_allowed: z.boolean(),
    max_lines: z.number().int().min(1).max(100),
    monthly_spend_limit: z.string().refine(
      (v) => v === "" || positiveCredits.safeParse(v).success,
      "Enter a positive credit amount or leave empty",
    ),
  })
  .refine((v) => !v.premium_allowed || v.elevenlabs_eua !== "", {
    path: ["elevenlabs_eua"],
    message: "Premium voices require an End User Agreement version",
  });
export type EligibilityForm = z.infer<typeof eligibilityFormSchema>;

const lineOperations = {
  requirement_reference: text(),
  registration_status: z.literal("approved", "Only approved regulatory status can be registered"),
  sms_sender_registration: optionalText(),
  supplier_tariffs: jsonArray(tariffInputSchema, "Enter a JSON list of supplier tariffs (1–16)"),
  call_slot_limit: z.number().int().min(1).max(100),
  calls_per_minute: z.number().int().min(1).max(1000),
  sms_per_minute: z.number().int().min(1).max(1000),
};
export const lineRegistrationFormSchema = z
  .object({
    line_id: z.uuid(),
    trusted_account_id: z.uuid("Choose a trusted account"),
    phone_number: z.string().regex(/^\+[1-9]\d{6,14}$/, "Use E.164, for example +6560000000"),
    country: z.string().regex(/^[A-Z]{2}$/, "Use an ISO country code such as SG"),
    number_resource_id: text(128),
    voice_application_id: optionalText(128),
    voice_billing_service_id: z.string(),
    sms_application_id: optionalText(128),
    sms_billing_service_id: z.string(),
    rental_billing_service_id: z.uuid("Choose the rental billing service"),
    standard_voice_options: z.string().refine(
      (v) => v.trim() === "" || voiceOptionsJson.safeParse(v).success,
      "Enter a JSON list of {voice, language} pairs or leave empty",
    ),
    ...lineOperations,
  })
  .refine((v) => v.voice_application_id !== "" || v.sms_application_id !== "", {
    path: ["voice_application_id"],
    message: "Register at least one capability",
  })
  .refine((v) => v.voice_application_id === "" || z.uuid().safeParse(v.voice_billing_service_id).success, {
    path: ["voice_billing_service_id"],
    message: "Choose the voice billing service",
  })
  .refine((v) => v.sms_application_id === "" || z.uuid().safeParse(v.sms_billing_service_id).success, {
    path: ["sms_billing_service_id"],
    message: "Choose the SMS billing service",
  })
  .refine((v) => v.sms_application_id === "" || v.sms_sender_registration !== "", {
    path: ["sms_sender_registration"],
    message: "SMS requires a sender registration reference",
  });
export type LineRegistrationForm = z.infer<typeof lineRegistrationFormSchema>;
export const lineUpdateFormSchema = z.object(lineOperations);
export type LineUpdateForm = z.infer<typeof lineUpdateFormSchema>;

export const UNDERWRITING_WORDING =
  "Telnyx managed voice is an underwritten exception, not reserved liability.";
export const underwritingFormSchema = z
  .object({
    enabled: z.boolean(),
    acceptance_text: text(4096),
    acceptance_version: text(128),
    paid_tier_evidence_reference: text(4096),
    max_leg_seconds: z.number().int().min(1801, "The ceiling must exceed the 1,800-second call limit"),
    ceiling_kind: z.enum(["written_confirmation", "paid_inbound_test"]),
    ceiling_reference: text(4096),
    hangup_cause: z.string(),
    paid_tier: z.boolean(),
    timestamp_fixture_reference: optionalText(4096),
    detail_record_options_reference: text(4096),
    detail_record_fixture_reference: text(4096),
    detail_record_fields: z.string(),
    supplier_tariff_revision: z.number().int().positive(),
    fx_policy_revision: text(128),
    character_tariff_reference: text(4096),
    components: jsonArray(
      z.object({
        kind: z.enum(["carrier_inbound", "relay", "stt", "tts", "premium_tts", "sms_part"]),
        credits_per_second: positiveCredits,
        increment_seconds: z.number().int().positive(),
        minimum_seconds: z.number().int().nonnegative(),
      }),
      "Enter a JSON list of per-second exposure components",
    ),
    rolling_budget: positiveCredits,
  })
  .refine(
    (v) =>
      v.ceiling_kind === "written_confirmation" ||
      (v.paid_tier && v.hangup_cause === "time_limit"),
    {
      path: ["paid_tier"],
      message: "Only a paid-tier inbound test ending with time_limit is valid evidence",
    },
  );
export type UnderwritingForm = z.infer<typeof underwritingFormSchema>;

export const voiceAccountFormSchema = z
  .object({
    per_call_characters: z.number().int().min(700).max(1_000_000),
    greeting: optionalText(4096),
    wrapup: optionalText(300),
    premium: z.boolean(),
    api_key_ref: z.string(),
    voices: z.string(),
    billing_account_identity: z.string(),
    monthly_characters: z.number().int(),
    plan_tier: z.enum(["Scale", "Business", "Enterprise"]),
    oem_terms_version: z.string(),
    admin_acknowledged: z.boolean(),
    billing_service_id: z.string(),
  })
  .refine((v) => !v.premium || /^[A-Za-z0-9_\-./:]{1,256}$/.test(v.api_key_ref), {
    path: ["api_key_ref"],
    message: "Enter the Telnyx integration secret identifier",
  })
  .refine((v) => !v.premium || jsonArray(managedVoiceOptionSchema, "").safeParse(v.voices).success, {
    path: ["voices"],
    message: "Enter a JSON list of ElevenLabs {voice, language} pairs",
  })
  .refine((v) => !v.premium || /^[A-Za-z0-9_\-./:]{1,256}$/.test(v.billing_account_identity), {
    path: ["billing_account_identity"],
    message: "Enter the ElevenLabs billing account identity",
  })
  .refine((v) => !v.premium || /^[A-Za-z0-9_\-./:]{1,256}$/.test(v.oem_terms_version), {
    path: ["oem_terms_version"],
    message: "Enter the OEM terms version",
  })
  .refine((v) => !v.premium || v.monthly_characters >= v.per_call_characters, {
    path: ["monthly_characters"],
    message: "The monthly budget must cover at least one call",
  })
  .refine((v) => !v.premium || v.admin_acknowledged, {
    path: ["admin_acknowledged"],
    message: "Acknowledge the ElevenLabs plan and OEM terms",
  })
  .refine((v) => !v.premium || z.uuid().safeParse(v.billing_service_id).success, {
    path: ["billing_service_id"],
    message: "Choose the premium billing service",
  });
export type VoiceAccountForm = z.infer<typeof voiceAccountFormSchema>;

export const costPolicyFormSchema = z.object({
  fx: jsonArray(
    z.object({
      currency: z.string().regex(/^[A-Z]{3}$/),
      credits_per_unit: positiveCredits,
      revision: z.string().min(1),
    }),
    "Enter a JSON list of {currency, credits_per_unit, revision}",
  ),
  schemas: jsonArray(
    z.object({
      kind: z.string(),
      record_type: z.string(),
      amount_field: z.string(),
      currency_field: z.string(),
      options_reference: z.string(),
      fixture_reference: z.string(),
    }),
    "Enter a JSON list of attested detail-record schemas",
  ),
  inventory: z.boolean(),
  cycle_anchor: z.string(),
  amount: z.string(),
  currency: z.string(),
  source: z.string(),
  fx_policy_revision: z.string(),
}).refine((v) => !v.inventory || !Number.isNaN(Date.parse(v.cycle_anchor)), {
  path: ["cycle_anchor"],
  message: "Enter an RFC 3339 time such as 2026-11-01T00:00:00Z",
}).refine((v) => !v.inventory || positiveCredits.safeParse(v.amount).success, {
  path: ["amount"],
  message: "Enter a positive credit amount",
});
export type CostPolicyForm = z.infer<typeof costPolicyFormSchema>;

export const evidenceFormSchema = z.object({
  session_id: z.uuid("Enter the call session ID"),
  provider_seconds: z.number().int().min(0).max(86400),
  provider_reference: text(4096),
});
export type EvidenceForm = z.infer<typeof evidenceFormSchema>;

/** API instants are RFC 3339 strings; exact credits are decimal strings. */
export function rfc3339(value: string): string {
  return new Date(value).toISOString();
}
