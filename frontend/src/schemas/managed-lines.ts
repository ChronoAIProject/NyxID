import { z } from "zod";
import { decimalCredits, parseCredits } from "@/lib/credits";

/** Decimal credit strings are exact; never parse them as floating point. */
const credits = z.string().refine((value) => {
  try {
    parseCredits(value);
    return true;
  } catch {
    return false;
  }
}, "Expected an exact credit amount");

export const managedTermsSchema = z.object({
  managed_line: z.string(),
  elevenlabs_eua: z.string().nullable(),
});
export type ManagedTerms = z.infer<typeof managedTermsSchema>;

export const managedEligibilitySchema = z.object({
  id: z.uuid(),
  owner_id: z.uuid(),
  business_legal_name: z.string(),
  registration_id: z.string(),
  jurisdiction: z.string(),
  allowed_use_case: z.string(),
  sms_sender_path: z.string(),
  sms_registration_status: z.string(),
  required_terms: managedTermsSchema,
  accepted_terms: managedTermsSchema.nullable(),
  terms_accepted_at: z.string().nullable(),
  premium_allowed: z.boolean(),
  max_lines: z.number().int(),
  monthly_spend_limit: credits.nullable(),
  revision: z.number().int(),
  approved_at: z.string(),
  revoked_at: z.string().nullable(),
});
export type ManagedEligibility = z.infer<typeof managedEligibilitySchema>;

export function termsAccepted(e: ManagedEligibility): boolean {
  return (
    e.accepted_terms?.managed_line === e.required_terms.managed_line &&
    (e.accepted_terms?.elevenlabs_eua ?? null) === e.required_terms.elevenlabs_eua
  );
}

export const managedVoiceOptionSchema = z.object({
  voice: z.string(),
  language: z.string(),
});
export type ManagedVoiceOption = z.infer<typeof managedVoiceOptionSchema>;
export const isPremiumVoice = (o: ManagedVoiceOption) =>
  o.voice.startsWith("ElevenLabs.");
export const voiceKey = (o: ManagedVoiceOption) => `${o.voice}|${o.language}`;

export const managedRateSchema = z.object({
  metric: z.string(),
  credits_per_unit: credits,
  revision: z.string(),
  /** Older servers omit purpose; the UI then shows the metric only. */
  purpose: z
    .enum(["rental", "calls", "sms", "premium_voice"])
    .or(z.string())
    .optional(),
});
export type ManagedRate = z.infer<typeof managedRateSchema>;

export const managedQuoteKindSchema = z.union([
  z.object({ kind: z.literal("claim") }),
  z.object({
    kind: z.literal("renewal"),
    claim_id: z.uuid(),
    generation: z.number().int(),
    period_index: z.number().int(),
  }),
  z.object({
    kind: z.literal("resume"),
    claim_id: z.uuid(),
    generation: z.number().int(),
  }),
]);
export type ManagedQuoteKind = z.infer<typeof managedQuoteKindSchema>;

export const managedQuoteSchema = z.object({
  id: z.uuid(),
  line_id: z.uuid(),
  kind: managedQuoteKindSchema,
  rental: managedRateSchema,
  usage: z.array(managedRateSchema),
  standard_voice_options: z.array(managedVoiceOptionSchema),
  terms: managedTermsSchema,
  accept_before: z.string(),
  period_days: z.number().int(),
  paid_period_refundable: z.boolean(),
});
export type ManagedQuote = z.infer<typeof managedQuoteSchema>;

/** Prepaid rental for one period, computed exactly in picocredits. */
export function periodRental(quote: ManagedQuote): string {
  return decimalCredits(
    parseCredits(quote.rental.credits_per_unit) * BigInt(quote.period_days),
  );
}
export function perMinute(rate: string): string {
  return decimalCredits(parseCredits(rate) * 60n);
}

export const managedClaimSchema = z.object({
  id: z.uuid(),
  generation: z.number().int(),
  state: z.string(),
  suspension: z.string().nullable(),
  auto_renew: z.boolean(),
  standard_voice: managedVoiceOptionSchema.nullable(),
  covered_from: z.string().nullish(),
  covered_until: z.string().nullable(),
  period_index: z.number().int().nullish(),
  renewal_quote: managedQuoteSchema.nullable(),
  agent_id: z.string().optional(),
  voice_tier: z.enum(["standard", "premium"]).optional(),
  sms_enabled: z.boolean().optional(),
  caller_confirmations: z.boolean().optional(),
  voice_binding_id: z.uuid().nullish(),
  voice_binding_revision: z.number().int().nullish(),
  sms_binding_id: z.uuid().nullish(),
  release_requested_at: z.string().nullish(),
  suspended_at: z.string().nullish(),
});
export type ManagedClaim = z.infer<typeof managedClaimSchema>;

export const managedLineSchema = z.object({
  id: z.uuid(),
  phone_number: z.string(),
  country: z.string(),
  state: z.string(),
  voice: z.boolean(),
  standard_voice_options: z.array(managedVoiceOptionSchema),
  sms: z.boolean(),
  claim: managedClaimSchema.nullable(),
});
export type ManagedLine = z.infer<typeof managedLineSchema>;

export const managedLineListSchema = z.object({
  lines: z.array(managedLineSchema),
  next_after_id: z.string().nullish(),
});

export const managedLifecycleSchema = z.object({
  request_id: z.uuid(),
  kind: z.string(),
  line_id: z.uuid(),
  claim_id: z.uuid().nullable(),
  period_id: z.uuid().nullable(),
});
export type ManagedLifecycle = z.infer<typeof managedLifecycleSchema>;

/**
 * Explicit merchant consent: the quoted price and the current terms versions
 * (plus the ElevenLabs End User Agreement for premium) are checked separately.
 */
export const managedClaimFormSchema = z
  .object({
    agent_id: z.uuid("Choose a specialist"),
    voice_tier: z.enum(["standard", "premium"]),
    voice_option: z.string(),
    enable_sms: z.boolean(),
    caller_confirmations: z.boolean(),
    auto_renew: z.boolean(),
    accept_price: z.boolean().refine(Boolean, "Accept the quoted prices"),
    accept_terms: z.boolean().refine(Boolean, "Accept the managed line terms"),
    accept_eua: z.boolean(),
  })
  .refine((v) => !v.caller_confirmations || v.enable_sms, {
    path: ["caller_confirmations"],
    message: "Caller confirmations require SMS",
  })
  .refine((v) => v.voice_tier !== "premium" || v.accept_eua, {
    path: ["accept_eua"],
    message: "Accept the ElevenLabs End User Agreement",
  });
export type ManagedClaimForm = z.infer<typeof managedClaimFormSchema>;

export const managedCoverageFormSchema = z.object({
  auto_renew: z.boolean(),
  accept_price: z.boolean().refine(Boolean, "Accept the quoted prices"),
  accept_terms: z.boolean().refine(Boolean, "Accept the managed line terms"),
});
export type ManagedCoverageForm = z.infer<typeof managedCoverageFormSchema>;

export const managedTermsFormSchema = z.object({
  accept_terms: z.boolean().refine(Boolean, "Accept the managed line terms"),
  accept_eua: z.boolean(),
});
export type ManagedTermsForm = z.infer<typeof managedTermsFormSchema>;

/** Request IDs are UUID v4 and stay fixed while a submission is retried. */
export function newRequestId(): string {
  return crypto.randomUUID();
}
