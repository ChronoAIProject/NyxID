import { describe, expect, it } from "vitest";
import { cmaSandboxSchema, cmaSandboxListSchema } from "./nyxbot-cma-sandbox";

describe("CMA environment responses", () => {
  const pending = {
    id: "local",
    label: "Personal workspace",
    cma_sandbox_id: null,
    generation: 1,
    occupies_account_slot: true,
    admission_state: "creating",
    observation_state: "unknown",
    resource_state: null,
    machine_included: null,
    cpu_millis: null,
    memory_mib: null,
    last_activity_at: null,
    last_observed: null,
    retire_requested_at: null,
    retired_at: null,
    billing: { provider: "cma", cost_status: "unknown" },
  };
  it("preserves unknown capacity and pending occupancy without storing private fields", () => {
    const result = cmaSandboxSchema.parse({
      ...pending,
      request_ciphertext: "secret",
      agents: ["private"],
    });
    expect(result.cpu_millis).toBeNull();
    expect(result.occupies_account_slot).toBe(true);
    expect(result).not.toHaveProperty("request_ciphertext");
    expect(result).not.toHaveProperty("agents");
  });
  it("accepts complete empty, retired and conflicting inventories", () => {
    expect(
      cmaSandboxListSchema.parse({
        sandboxes: [],
        conflict: false,
        inventory_complete: true,
      }).sandboxes,
    ).toEqual([]);
    const retired = cmaSandboxSchema.parse({
      ...pending,
      occupies_account_slot: false,
      observation_state: "retired",
      admission_state: "retired",
      retired_at: "2026-11-04T00:00:00Z",
    });
    expect(retired.occupies_account_slot).toBe(false);
    expect(
      cmaSandboxListSchema.parse({
        sandboxes: [pending, { ...pending, id: "second" }],
        conflict: true,
        inventory_complete: true,
      }).sandboxes,
    ).toHaveLength(2);
  });
  it("rejects unsafe numeric resource values", () => {
    expect(
      cmaSandboxSchema.safeParse({ ...pending, cpu_millis: -1 }).success,
    ).toBe(false);
  });
});
