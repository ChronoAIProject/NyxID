import { describe, expect, it } from "vitest";
import {
  chainVerificationResponseSchema,
  chainVerifyStatusSchema,
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
  it("parses a healthy status", () => {
    const parsed = chainVerifyStatusSchema.parse(baseStatus);
    expect(parsed.outcome).toBe("ok");
    expect(parsed.anchor_seq).toBe(41);
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
    const parsed = chainVerificationResponseSchema.parse({ chains: [] });
    expect(parsed.chains).toHaveLength(0);
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
