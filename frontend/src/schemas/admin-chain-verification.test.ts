import { z } from "zod";
import { describe, expect, it } from "vitest";
import {
  chainVerificationResponseSchema,
  chainVerifyStatusSchema,
  startupDiagnosticSchema,
} from "./admin-chain-verification";

const baseStatus = {
  chain: "billing_ledger",
  outcome: "ok",
  cursor_seq: 42,
  head_seq: 41,
  checked_entries: 41,
  last_full_pass_at: "2026-08-03T10:00:00Z",
  break_seq: null,
  break_kind: null,
  break_detail: null,
  anchor_seq: 41,
  anchor_valid: true,
  pre_chain_count: null,
  last_run_at: "2026-08-03T10:05:00Z",
};

describe("admin chain verification schemas", () => {
  it("retains additive presentation fields and accepts legacy diagnostics", () => {
    const legacy = {
      code: "billing_hold_backlog",
      summary: "Unfinished billing reservations by status and age",
      detail: "7 requests are holding funds.",
      remediation: "This inventory is read-only.",
      detected_at: "2026-11-17T01:00:00Z",
    };
    expect(startupDiagnosticSchema.parse(legacy)).toEqual(legacy);
    for (const severity of ["info", "action"]) {
      const current = { ...legacy, severity, technical_details: "complete=true" };
      expect(startupDiagnosticSchema.parse(current)).toEqual(current);
      expect(startupDiagnosticSchema.omit({ severity: true, technical_details: true }).parse(current)).toEqual(legacy);
    }
  });
  it("parses a healthy status", () => {
    const parsed = chainVerifyStatusSchema.parse(baseStatus);
    expect(parsed.outcome).toBe("ok");
    expect(parsed.anchor_seq).toBe(41);
  });

  it("parses an active startup diagnostic", () => {
    const parsed = chainVerificationResponseSchema.parse({
      chains: [baseStatus],
      startup_diagnostics: [
        {
          code: "exact_service_semantic_effect_index",
          summary: "Semantic uniqueness is not enforced.",
          detail: "Two duplicate groups were detected.",
          remediation: "Reconcile duplicate groups and restart.",
          detected_at: "2026-08-25T10:00:00Z",
        },
      ],
    });
    expect(parsed.startup_diagnostics[0]?.code).toBe(
      "exact_service_semantic_effect_index",
    );
  });

  it("parses a broken status with break details", () => {
    const parsed = chainVerifyStatusSchema.parse({
      ...baseStatus,
      chain: "audit_log",
      outcome: "broken",
      break_seq: 7,
      break_kind: "hash_mismatch",
      break_detail: "expected abc actual def",
      anchor_seq: null,
      anchor_valid: null,
      pre_chain_count: 120,
    });
    expect(parsed.outcome).toBe("broken");
    expect(parsed.break_seq).toBe(7);
  });

  it("tolerates chains the sweep has not covered yet", () => {
    const parsed = chainVerificationResponseSchema.parse({
      chains: [],
      startup_diagnostics: [],
    });
    expect(parsed.chains).toHaveLength(0);
  });

  it("defaults diagnostics for responses served by an older replica", () => {
    const parsed = chainVerificationResponseSchema.parse({ chains: [] });
    expect(parsed.startup_diagnostics).toEqual([]);
  });

  it("rejects unknown outcomes and chains", () => {
    expect(() =>
      chainVerifyStatusSchema.parse({ ...baseStatus, outcome: "maybe" }),
    ).toThrow();
    expect(() =>
      chainVerifyStatusSchema.parse({ ...baseStatus, chain: "wallet" }),
    ).toThrow();
  });
});

it("adds account verification without changing the cached client's chain enum", () => {
  const response = {
    chains: [baseStatus],
    accounts: { ...baseStatus, chain: "billing_accounts" },
  };
  expect(chainVerificationResponseSchema.parse(response).accounts?.chain).toBe(
    "billing_accounts",
  );
  const cachedSchema = z.object({
    chains: z.array(
      chainVerifyStatusSchema.extend({
        chain: z.enum(["audit_log", "billing_ledger"]),
      }),
    ),
  });
  expect(cachedSchema.parse(response).chains).toHaveLength(1);
  expect(() =>
    chainVerificationResponseSchema.parse({ chains: [response.accounts] }),
  ).toThrow();
});


it("preserves bounded relay health and accepts an older replica without it", () => {
  expect(chainVerificationResponseSchema.parse({ chains: [] }).security_effects).toBeUndefined();
  const health = {
    counts: [{ kind: "audit", state: "pending", count: 1000, at_least: true }, { kind: "cae", state: "uncertain", count: 1, at_least: false }],
    oldest_pending_at: "2026-11-15T10:00:00Z",
    oldest_pending_age_seconds: 7200,
  };
  expect(chainVerificationResponseSchema.parse({ chains: [], security_effects: health }).security_effects).toEqual(health);
});
