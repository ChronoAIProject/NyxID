import { describe, expect, it } from "vitest";
import {
  billingConfigSchema,
  billingOverviewSchema,
  billingSettingsFormSchema,
  billingSettingsPatch,
} from "./admin-billing";

const config = {
  billing_enabled: true,
  lago_configured: false,
  resale_enabled: false,
  default_overdraft_cap_credits: {
    effective: 5,
    source: "env",
    global: null,
    env: 5,
  },
  max_default_overdraft_cap_credits: 1_000_000_000_000,
  environment: [
    {
      key: "LAGO_PLAN_CODE",
      value: "starter",
      source: "env",
      secret_set: null,
    },
    { key: "LAGO_API_KEY", value: null, source: "env", secret_set: true },
  ],
};

describe("admin billing schemas", () => {
  it("keeps the environment mirror and the effective cap source", () => {
    const parsed = billingConfigSchema.parse(config);
    expect(parsed.environment).toHaveLength(2);
    expect(parsed.environment[1]).toEqual({
      key: "LAGO_API_KEY",
      value: null,
      source: "env",
      secret_set: true,
    });
    expect(parsed.default_overdraft_cap_credits.source).toBe("env");
  });

  it("rejects a source outside the server vocabulary", () => {
    expect(() =>
      billingConfigSchema.parse({
        ...config,
        default_overdraft_cap_credits: {
          ...config.default_overdraft_cap_credits,
          source: "global or environment",
        },
      }),
    ).toThrow();
  });

  it("parses per-class effective prices", () => {
    const [row] = billingOverviewSchema.parse([
      {
        id: "svc-1",
        slug: "svc",
        name: "Service",
        is_active: true,
        effective_metric: "requests",
        allowance_metrics: ["requests"],
        effective: {
          fallback_metric: "requests",
          resale: { configured: false, applied: false, source: "free" },
          classes: [
            {
              credential_classes: ["user_owned", "node_managed"],
              lane: "byok",
              platform_charged: true,
              source: "service",
              reason: "lane_synced",
              prices: [
                {
                  layer: "platform",
                  metric: "requests",
                  operation: null,
                  lago_metric_code: "svc_byok",
                  credits_per_unit: "0.25",
                  source: "service",
                  origin: "lane_primary",
                  sync_status: "synced",
                  sync_error: null,
                  cached_rate: null,
                },
              ],
              pending: [],
            },
          ],
        },
      },
    ]);
    expect(row!.effective.classes[0]!.prices[0]!.credits_per_unit).toBe("0.25");
  });

  it("validates the settings form against the server bound", () => {
    const schema = billingSettingsFormSchema(100);
    expect(
      schema.safeParse({ default_overdraft_cap_credits: "" }).success,
    ).toBe(true);
    expect(
      schema.safeParse({ default_overdraft_cap_credits: "100" }).success,
    ).toBe(true);
    for (const bad of ["101", "-1", "1.5", "abc"]) {
      const result = schema.safeParse({ default_overdraft_cap_credits: bad });
      expect(result.success, bad).toBe(false);
    }
  });

  it("maps an empty cap to a clear and a number to a set", () => {
    expect(
      billingSettingsPatch({ default_overdraft_cap_credits: " " }),
    ).toEqual({
      default_overdraft_cap_credits: null,
    });
    expect(
      billingSettingsPatch({ default_overdraft_cap_credits: "37" }),
    ).toEqual({
      default_overdraft_cap_credits: 37,
    });
  });
});
