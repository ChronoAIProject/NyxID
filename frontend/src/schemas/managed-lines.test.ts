import { describe, expect, it } from "vitest";
import {
  isPremiumVoice,
  managedClaimFormSchema,
  managedEligibilitySchema,
  managedLineListSchema,
  managedQuoteSchema,
  perMinute,
  periodRental,
  termsAccepted,
  type ManagedClaimForm,
} from "./managed-lines";
import {
  costPolicyFormSchema,
  rfc3339,
  eligibilityFormSchema,
  lineRegistrationFormSchema,
  managedEffectPageSchema,
  managedReportSchema,
  underwritingFormSchema,
  voiceAccountFormSchema,
  type EligibilityForm,
  type LineRegistrationForm,
  type UnderwritingForm,
} from "./admin-managed-lines";

const ids = {
  owner: "3f0b8a3e-6c55-4b1e-9a61-1c2f0e9a7d10",
  line: "0b8f4b44-3d73-4d0c-8f59-5d1c4f7b2a61",
  claim: "8f8a6e93-1f0c-4f3e-9d77-2a8b9b0b6c3a",
  quote: "a7c0f5d2-5b8e-4d3a-8c5f-0e1d2c3b4a59",
  binding: "c3d2e1f0-9a8b-4c7d-8e6f-5a4b3c2d1e0f",
  agent: "d1e2f3a4-b5c6-4d7e-8f90-a1b2c3d4e5f6",
};
const quote = {
  id: ids.quote,
  line_id: ids.line,
  kind: { kind: "renewal", claim_id: ids.claim, generation: 2, period_index: 1 },
  rental: { metric: "phone_number_days", credits_per_unit: "0.333333333333", revision: "r1", purpose: "rental" },
  usage: [
    { metric: "voice_seconds", credits_per_unit: "0.0005", revision: "r1", purpose: "calls" },
    { metric: "requests", credits_per_unit: "0.02", revision: "r1", purpose: "sms" },
    { metric: "voice_seconds", credits_per_unit: "0.001", revision: "r1", purpose: "premium_voice" },
  ],
  standard_voice_options: [
    { voice: "Telnyx.Ultra.Callie", language: "en-US" },
    { voice: "ElevenLabs.eleven_flash_v2_5.abc", language: "zh-CN" },
  ],
  terms: { managed_line: "2026-11", elevenlabs_eua: "eua-1" },
  accept_before: "2026-11-20T00:00:00+00:00",
  period_days: 30,
  paid_period_refundable: false,
};

describe("merchant managed line contract", () => {
  it("parses claimed lines with coverage, bindings and a renewal quote", () => {
    const list = managedLineListSchema.parse({
      lines: [
        {
          id: ids.line,
          phone_number: "+6560000000",
          country: "SG",
          state: "claimed",
          voice: true,
          standard_voice_options: [{ voice: "Telnyx.Ultra.Callie", language: "en-US" }],
          sms: true,
          claim: {
            id: ids.claim,
            generation: 2,
            state: "claimed",
            suspension: null,
            auto_renew: true,
            standard_voice: { voice: "Telnyx.Ultra.Callie", language: "en-US" },
            covered_from: "2026-11-01T00:00:00+00:00",
            covered_until: "2026-12-01T00:00:00+00:00",
            period_index: 0,
            renewal_quote: quote,
            agent_id: ids.agent,
            voice_tier: "standard",
            sms_enabled: true,
            caller_confirmations: false,
            voice_binding_id: ids.binding,
            voice_binding_revision: 3,
            sms_binding_id: null,
            release_requested_at: null,
            suspended_at: null,
          },
        },
      ],
      next_after_id: null,
    });
    expect(list.lines[0]?.claim?.voice_binding_revision).toBe(3);
    expect(list.lines[0]?.claim?.renewal_quote?.kind).toEqual(quote.kind);
  });

  it("tolerates older servers without the additive claim fields or purposes", () => {
    const older = managedLineListSchema.parse({
      lines: [
        {
          id: ids.line,
          phone_number: "+6560000000",
          country: "SG",
          state: "claimed",
          voice: true,
          standard_voice_options: [],
          sms: false,
          claim: {
            id: ids.claim,
            generation: 1,
            state: "suspended",
            suspension: "unpaid",
            auto_renew: false,
            standard_voice: null,
            covered_until: "2026-12-01T00:00:00+00:00",
            renewal_quote: null,
          },
        },
      ],
    });
    expect(older.lines[0]?.claim?.voice_binding_id).toBeUndefined();
    const rental: Record<string, unknown> = { ...quote.rental };
    delete rental.purpose;
    expect(managedQuoteSchema.parse({ ...quote, rental }).rental.purpose).toBeUndefined();
  });

  it("computes prepaid rental and per-minute prices exactly", () => {
    const q = managedQuoteSchema.parse(quote);
    expect(periodRental(q)).toBe("9.99999999999");
    expect(perMinute("0.000000000001")).toBe("0.00000000006");
    expect(q.standard_voice_options.filter(isPremiumVoice)).toHaveLength(1);
  });

  it("requires every current terms version, including the End User Agreement", () => {
    const base = {
      id: ids.claim,
      owner_id: ids.owner,
      business_legal_name: "Jolly Chili Pte Ltd",
      registration_id: "201912345K",
      jurisdiction: "SG",
      allowed_use_case: "restaurant bookings",
      sms_sender_path: "long_code",
      sms_registration_status: "approved",
      required_terms: { managed_line: "2026-11", elevenlabs_eua: "eua-1" },
      accepted_terms: { managed_line: "2026-11", elevenlabs_eua: null },
      terms_accepted_at: null,
      premium_allowed: true,
      max_lines: 1,
      monthly_spend_limit: "250",
      revision: 4,
      approved_at: "2026-11-01T00:00:00+00:00",
      revoked_at: null,
    };
    expect(termsAccepted(managedEligibilitySchema.parse(base))).toBe(false);
    expect(
      termsAccepted(
        managedEligibilitySchema.parse({ ...base, accepted_terms: base.required_terms }),
      ),
    ).toBe(true);
    expect(() =>
      managedEligibilitySchema.parse({ ...base, monthly_spend_limit: "1e3" }),
    ).toThrow();
  });

  it("refuses claims without explicit consent, SMS for confirmations or premium agreement", () => {
    const form: ManagedClaimForm = {
      agent_id: ids.agent,
      voice_tier: "standard",
      voice_option: "Telnyx.Ultra.Callie|en-US",
      enable_sms: true,
      caller_confirmations: true,
      auto_renew: true,
      accept_price: true,
      accept_terms: true,
      accept_eua: false,
    };
    expect(managedClaimFormSchema.safeParse(form).success).toBe(true);
    for (const change of [
      { accept_price: false },
      { accept_terms: false },
      { enable_sms: false },
      { voice_tier: "premium" as const },
      { agent_id: "" },
    ]) {
      expect(managedClaimFormSchema.safeParse({ ...form, ...change }).success).toBe(false);
    }
    expect(
      managedClaimFormSchema.safeParse({ ...form, voice_tier: "premium", accept_eua: true })
        .success,
    ).toBe(true);
  });
});

describe("admin managed line forms", () => {
  const registration: LineRegistrationForm = {
    line_id: ids.line,
    trusted_account_id: ids.owner,
    phone_number: "+6560000000",
    country: "SG",
    number_resource_id: "1234567890",
    voice_application_id: "app-voice",
    voice_billing_service_id: ids.binding,
    sms_application_id: "",
    sms_billing_service_id: "",
    rental_billing_service_id: ids.quote,
    standard_voice_options: "",
    requirement_reference: "bundle-1",
    registration_status: "approved",
    sms_sender_registration: "",
    supplier_tariffs: JSON.stringify([
      {
        kind: "carrier_inbound",
        amount: "0.0085",
        currency: "USD",
        unit: "minute",
        increment: 60,
        minimum: 60,
        source: "https://supplier.example/pricing",
        revision: 1,
        fx_policy_revision: "fx-1",
        effective_at: "2026-11-01T00:00:00Z",
      },
    ]),
    call_slot_limit: 2,
    calls_per_minute: 10,
    sms_per_minute: 10,
  };

  it("validates registration capabilities, sender registration and tariff evidence", () => {
    expect(lineRegistrationFormSchema.safeParse(registration).success).toBe(true);
    for (const change of [
      { voice_application_id: "" },
      { phone_number: "6560000000" },
      { sms_application_id: "msg-profile", sms_billing_service_id: ids.claim },
      { supplier_tariffs: "[{\"kind\":\"carrier_inbound\"}]" },
      { supplier_tariffs: "not json" },
      { registration_status: "pending" },
      { voice_billing_service_id: "" },
    ]) {
      expect(lineRegistrationFormSchema.safeParse({ ...registration, ...change }).success).toBe(
        false,
      );
    }
    expect(
      lineRegistrationFormSchema.safeParse({
        ...registration,
        sms_application_id: "msg-profile",
        sms_billing_service_id: ids.claim,
        sms_sender_registration: "sender-1",
      }).success,
    ).toBe(true);
  });

  it("keeps the underwriting evidence rules of the server", () => {
    const form: UnderwritingForm = {
      enabled: true,
      acceptance_text: "Telnyx managed voice is an underwritten exception, not reserved liability.",
      acceptance_version: "v1",
      paid_tier_evidence_reference: "invoice",
      max_leg_seconds: 14400,
      ceiling_kind: "paid_inbound_test",
      ceiling_reference: "call",
      hangup_cause: "time_limit",
      paid_tier: true,
      timestamp_fixture_reference: "",
      detail_record_options_reference: "options",
      detail_record_fixture_reference: "fixture",
      detail_record_fields: "call_leg_id, connection_id",
      supplier_tariff_revision: 1,
      fx_policy_revision: "fx-1",
      character_tariff_reference: "tts",
      components: JSON.stringify([
        { kind: "carrier_inbound", credits_per_second: "0.0002", increment_seconds: 60, minimum_seconds: 60 },
      ]),
      rolling_budget: "500",
    };
    expect(underwritingFormSchema.safeParse(form).success).toBe(true);
    for (const change of [
      { paid_tier: false },
      { hangup_cause: "normal_clearing" },
      { max_leg_seconds: 1800 },
      { rolling_budget: "0" },
      { components: "[{\"kind\":\"relay\",\"credits_per_second\":\"-1\"}]" },
    ]) {
      expect(underwritingFormSchema.safeParse({ ...form, ...change }).success).toBe(false);
    }
    expect(
      underwritingFormSchema.safeParse({ ...form, ceiling_kind: "written_confirmation", paid_tier: false })
        .success,
    ).toBe(true);
  });

  it("requires premium agreement versions and positive spend limits", () => {
    const form: EligibilityForm = {
      owner_id: ids.owner,
      business_legal_name: "Jolly Chili Pte Ltd",
      registration_id: "201912345K",
      jurisdiction: "SG",
      allowed_use_case: "restaurant bookings",
      sms_sender_path: "long_code",
      sms_registration_status: "approved",
      managed_line_terms: "2026-11",
      elevenlabs_eua: "",
      premium_allowed: false,
      max_lines: 1,
      monthly_spend_limit: "",
    };
    expect(eligibilityFormSchema.safeParse(form).success).toBe(true);
    expect(eligibilityFormSchema.safeParse({ ...form, premium_allowed: true }).success).toBe(false);
    expect(eligibilityFormSchema.safeParse({ ...form, monthly_spend_limit: "0" }).success).toBe(false);
    expect(eligibilityFormSchema.safeParse({ ...form, jurisdiction: "sg" }).success).toBe(false);
  });

  it("checks premium voice configuration only when premium is offered", () => {
    const base = {
      per_call_characters: 12000,
      greeting: "",
      wrapup: "",
      premium: false,
      api_key_ref: "",
      voices: "",
      billing_account_identity: "",
      monthly_characters: 0,
      plan_tier: "Scale" as const,
      oem_terms_version: "",
      admin_acknowledged: false,
      billing_service_id: "",
    };
    expect(voiceAccountFormSchema.safeParse(base).success).toBe(true);
    expect(voiceAccountFormSchema.safeParse({ ...base, premium: true }).success).toBe(false);
    expect(
      voiceAccountFormSchema.safeParse({
        ...base,
        premium: true,
        api_key_ref: "elevenlabs-key",
        voices: JSON.stringify([{ voice: "ElevenLabs.eleven_flash_v2_5.abc", language: "en-US" }]),
        billing_account_identity: "el-account",
        monthly_characters: 1_000_000,
        oem_terms_version: "eua-1",
        admin_acknowledged: true,
        billing_service_id: ids.quote,
      }).success,
    ).toBe(true);
  });

  it("sends RFC 3339 anchors and decimal strings, never BSON extended JSON", () => {
    expect(rfc3339("2026-11-01T08:00:00+08:00")).toBe("2026-11-01T00:00:00.000Z");
    const policy = {
      fx: "[]",
      schemas: "[]",
      inventory: true,
      cycle_anchor: "2026-11-01T00:00:00Z",
      amount: "1.2",
      currency: "USD",
      source: "https://supplier.example/rental",
      fx_policy_revision: "fx-1",
    };
    expect(costPolicyFormSchema.safeParse(policy).success).toBe(true);
    for (const change of [{ cycle_anchor: "yesterday" }, { amount: "1e3" }]) {
      expect(costPolicyFormSchema.safeParse({ ...policy, ...change }).success).toBe(false);
    }
    expect(
      costPolicyFormSchema.safeParse({ ...policy, inventory: false, cycle_anchor: "" }).success,
    ).toBe(true);
    const components = JSON.stringify([
      { kind: "relay", credits_per_second: { $numberDecimal: "0.0002" }, increment_seconds: 60, minimum_seconds: 60 },
    ]);
    expect(
      underwritingFormSchema.safeParse({
        enabled: true,
        acceptance_text: "Telnyx managed voice is an underwritten exception, not reserved liability.",
        acceptance_version: "v1",
        paid_tier_evidence_reference: "invoice",
        max_leg_seconds: 14400,
        ceiling_kind: "written_confirmation",
        ceiling_reference: "letter",
        hangup_cause: "",
        paid_tier: false,
        timestamp_fixture_reference: "",
        detail_record_options_reference: "options",
        detail_record_fixture_reference: "fixture",
        detail_record_fields: "call_leg_id",
        supplier_tariff_revision: 1,
        fx_policy_revision: "fx-1",
        character_tariff_reference: "tts",
        components,
        rolling_budget: "500",
      }).success,
    ).toBe(false);
  });

  it("parses the report and effect pages, including older effect rows", () => {
    expect(
      managedReportSchema.parse({
        lines: [
          {
            line_id: ids.line,
            days: [
              {
                day: "2026-11-01",
                billed_credits: "3.5",
                provider_actual_cost_credits: "0",
                estimated_cost_credits: "1.25",
                inventory_cost_credits: null,
                estimated_margin_credits: "2.25",
                actual_margin_credits: null,
                unknown_components: 1,
                absorbed_seconds: 0,
                absorbed_sms_parts: 0,
                effect_count: 4,
              },
            ],
            budgets: [{ kind: "monthly_spend", limit: "100", recognized: "30", unresolved_holds: "0.9" }],
            inventory_policy_missing: true,
          },
        ],
        next_after: null,
      }).lines[0]?.days[0]?.actual_margin_credits,
    ).toBeNull();
    const page = managedEffectPageSchema.parse({
      effects: [
        {
          id: "row",
          effect_id: ids.claim,
          component_coverage: false,
          evidence_quality: "estimated",
          original_costs: [],
        },
      ],
      next_after: null,
    });
    expect(page.effects[0]?.kind).toBeUndefined();
  });
});
