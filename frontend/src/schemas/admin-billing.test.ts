import { describe, expect, it } from "vitest";
import { billingConfigSchema, billingOverviewSchema } from "./admin-billing";

describe("admin billing schemas", () => {
  it("accepts redacted config and explicit precedence", () => {
    const value = billingConfigSchema.parse({ billing_enabled: true, lago_configured: false, resale_enabled: false, global: null, precedence: ["env", "global", "service", "legacy/free"] });
    expect(value.precedence).toContain("service");
  });
  it("requires effective source on overview rows", () => {
    expect(() => billingOverviewSchema.parse([{ id: "1", slug: "svc", name: "Service", has_billing: true, effective_source: "service" }])).not.toThrow();
  });
});
