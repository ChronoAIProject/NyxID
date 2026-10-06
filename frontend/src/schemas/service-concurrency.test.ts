import { describe, expect, it } from "vitest";
import { concurrencyPolicySchema } from "./service-concurrency";
const id = "aabbccdd-0000-4000-8000-000000000001";
const policy = { default_limit: 2, users: [], orgs: [] };
describe("concurrency policy", () => {
  it("supports unlimited defaults and overrides", () => {
    expect(concurrencyPolicySchema.parse({ ...policy, default_limit: null, users: [{ id, limit: null }] }).users[0]?.limit).toBeNull();
  });
  it.each([0, -1, 1.5, 10001, "2"])("rejects invalid limit %s", (limit) => {
    expect(concurrencyPolicySchema.safeParse({ ...policy, default_limit: limit }).success).toBe(false);
  });
  it("validates IDs and duplicates across target kinds", () => {
    expect(concurrencyPolicySchema.safeParse({ ...policy, users: [{ id: "bad", limit: 1 }] }).success).toBe(false);
    expect(concurrencyPolicySchema.safeParse({ ...policy, users: [{ id, limit: 1 }], orgs: [{ id, limit: 8 }] }).success).toBe(false);
  });
  it("bounds the combined target list", () => {
    const users = Array.from({ length: 501 }, (_, i) => ({ id: `aabbccdd-0000-4000-8000-${String(i).padStart(12, "0")}`, limit: 1 }));
    expect(concurrencyPolicySchema.safeParse({ ...policy, users: users.slice(0, 500) }).success).toBe(true);
    expect(concurrencyPolicySchema.safeParse({ ...policy, users: users.slice(0, 300), orgs: users.slice(300) }).success).toBe(false);
  });
});
