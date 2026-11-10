import { describe, expect, it } from "vitest";
import {
  cmaSandboxSchema,
  cmaSandboxListSchema,
} from "./assistant-cma-sandbox";

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
  it("preserves the capacity source supplied with its values", () => {
    const capacity = {
      cpu_millis: 2000,
      memory_mib: 8192,
      source: "runtime_allocation",
    };
    expect(cmaSandboxSchema.parse({ ...pending, capacity })).toHaveProperty(
      "capacity",
      capacity,
    );
  });
  it("accepts older responses without assigning an unverified capacity source", () => {
    expect(cmaSandboxSchema.safeParse(pending).success).toBe(true);
    expect(
      cmaSandboxSchema.parse({
        ...pending,
        capacity: { cpu_millis: 2000, memory_mib: 8192 },
      }),
    ).toHaveProperty("capacity.source", null);
  });
  it.each([
    { cpu_millis: 2000, memory_mib: 8192, source: "estimated" },
    { cpu_millis: -1, memory_mib: 8192, source: "runtime_allocation" },
  ])("rejects an invalid capacity projection %j", (capacity) => {
    expect(cmaSandboxSchema.safeParse({ ...pending, capacity }).success).toBe(
      false,
    );
  });
  it("accepts complete empty, retired and conflicting inventories", () => {
    expect(
      cmaSandboxListSchema.parse({
        sandboxes: [],
        occupied: 0,
        limit: 1,
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
        occupied: 2,
        limit: 1,
        conflict: true,
        inventory_complete: true,
      }).sandboxes,
    ).toHaveLength(2);
  });
  it("preserves allocation totals independently of the rows in a page", () => {
    expect(
      cmaSandboxListSchema.parse({
        sandboxes: [],
        occupied: 1,
        limit: 1,
        conflict: false,
        inventory_complete: true,
      }),
    ).toMatchObject({ occupied: 1, limit: 1 });
  });
  it.each([
    { occupied: -1, limit: 1 },
    { occupied: 0.5, limit: 1 },
    { occupied: 0, limit: 0 },
    { occupied: 0, limit: 1.5 },
    { occupied: undefined, limit: 1 },
    { occupied: 0, limit: undefined },
  ])("rejects invalid allocation totals %j", (allocation) => {
    expect(
      cmaSandboxListSchema.safeParse({
        sandboxes: [],
        ...allocation,
        conflict: false,
        inventory_complete: true,
      }).success,
    ).toBe(false);
  });
  it("rejects unsafe numeric resource values", () => {
    expect(
      cmaSandboxSchema.safeParse({ ...pending, cpu_millis: -1 }).success,
    ).toBe(false);
  });
});
