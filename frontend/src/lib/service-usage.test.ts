import { describe, expect, it } from "vitest";
import type { BillingUsageRow } from "@/schemas/billing";
import { serviceUsageDaily, serviceUsageSummary } from "./service-usage";

const row = (overrides: Partial<BillingUsageRow>): BillingUsageRow => ({
  service_slug: "llm-deepseek",
  metric: "tokens",
  lago_metric_code: "platform_svc_llm-deepseek_pk",
  layer: "platform",
  quantity: 0,
  requests: 0,
  bytes: 0,
  events: 0,
  lago_acked: true,
  billable: true,
  ...overrides,
});

describe("serviceUsageSummary", () => {
  it("counts token-metered calls from events and splits funding", () => {
    const summary = serviceUsageSummary(
      [
        row({
          quantity: 300,
          events: 2,
          api_key_id: "k1",
          api_key_name: "heca",
          estimated_credits: "0.0003",
          grant_credits: "0.0003",
        }),
        row({
          quantity: 210,
          events: 1,
          estimated_credits: "0.00021",
          wallet_credits: "0.00021",
        }),
        row({ service_slug: "other", quantity: 99, events: 9 }),
      ],
      "llm-deepseek",
    );
    expect(summary).toMatchObject({
      calls: 3,
      quantities: [{ metric: "tokens", quantity: 510 }],
      charged: "0.00051",
      grant: "0.0003",
      wallet: "0.00021",
      allowance: "0",
      billable: true,
      agents: [
        { name: "heca", calls: 2 },
        { name: null, calls: 1 },
      ],
    });
  });

  it("reports metering-only usage without a charge and unsettled charges as unknown", () => {
    expect(
      serviceUsageSummary(
        [row({ billable: false, metric: "requests", quantity: 4, events: 4 })],
        "llm-deepseek",
      ),
    ).toMatchObject({ calls: 4, billable: false, charged: "0" });
    expect(
      serviceUsageSummary([row({ events: 1 })], "llm-deepseek")?.charged,
    ).toBeNull();
    expect(serviceUsageSummary([row({})], "api-twitter")).toBeNull();
  });
});

describe("serviceUsageDaily", () => {
  const now = new Date("2026-10-06T12:00:00Z");
  it("zero-fills each UTC day of the period, oldest first", () => {
    const series = serviceUsageDaily(
      [
        row({
          day: "2026-10-04T00:00:00Z",
          events: 2,
          quantity: 20,
          estimated_credits: "0.1",
        }),
        row({
          day: "2026-10-06T00:00:00Z",
          events: 1,
          quantity: 5,
          estimated_credits: "0.05",
        }),
      ],
      "llm-deepseek",
      "7d",
      now,
    );
    expect(series?.map((d) => [d.day, d.calls, d.charged])).toEqual([
      ["2026-09-30", 0, "0"],
      ["2026-10-01", 0, "0"],
      ["2026-10-02", 0, "0"],
      ["2026-10-03", 0, "0"],
      ["2026-10-04", 2, "0.1"],
      ["2026-10-05", 0, "0"],
      ["2026-10-06", 1, "0.05"],
    ]);
  });

  it("has no daily view for 24h or a server without day buckets", () => {
    expect(serviceUsageDaily([], "llm-deepseek", "24h", now)).toBeNull();
    expect(
      serviceUsageDaily([row({ events: 1 })], "llm-deepseek", "30d", now),
    ).toBeNull();
    expect(
      serviceUsageDaily(
        [row({ service_slug: "other" })],
        "llm-deepseek",
        "30d",
        now,
      ),
    ).toBeNull();
    expect(serviceUsageDaily([], "llm-deepseek", "30d", now)).toHaveLength(30);
  });
});
