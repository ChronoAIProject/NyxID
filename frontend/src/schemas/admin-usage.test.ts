import { describe, expect, it } from "vitest";
import {
  adminUsageResponseSchema,
  normalizeAdminUsageSearch,
  usageRangeError,
} from "./admin-usage";
import { metricLabel } from "./billing-metrics";
import { usageFixture } from "@/test/admin-usage-fixture";
import { usageStatsSchema } from "./admin-usage";

describe("admin usage contracts", () => {
  it("accepts older responses without image subsets while preserving reported totals", () => {
    const response = usageFixture();
    const { image_input_tokens, image_output_tokens, ...older } =
      response.totals;
    expect(image_input_tokens + image_output_tokens).toBe(0);
    expect(usageStatsSchema.parse(older)).toMatchObject({
      total_tokens: 120,
      image_input_tokens: 0,
      image_output_tokens: 0,
    });
  });
  it("validates totals, identities, costs and retains future metrics", () => {
    const response = usageFixture();
    response.totals.quantities.new_unit = 9;
    response.totals.gross_cost_micros = null;
    expect(
      adminUsageResponseSchema.parse(response).totals.quantities.new_unit,
    ).toBe(9);
    expect(metricLabel("new_unit")).toBe("new_unit");
    expect(
      adminUsageResponseSchema.safeParse({
        ...response,
        totals: { ...response.totals, requests: "7" },
      }).success,
    ).toBe(false);
    expect(
      adminUsageResponseSchema.safeParse({
        ...response,
        ranking: [{ ...response.ranking[0], user: { id: "id" } }],
      }).success,
    ).toBe(false);
  });
  it("reads per-operation service usage and defaults it for older servers", () => {
    const response = usageFixture();
    const { by_operation: _omitted, ...older } = response.by_service[0]!;
    void _omitted;
    expect(
      adminUsageResponseSchema.parse({ ...response, by_service: [older] })
        .by_service[0]!.by_operation,
    ).toEqual([]);
    const priced = adminUsageResponseSchema.parse({
      ...response,
      by_service: [
        {
          ...response.by_service[0],
          by_operation: [{ ...usageFixture().totals, operation: "get_me" }],
        },
      ],
    });
    expect(priced.by_service[0]!.by_operation[0]).toMatchObject({
      operation: "get_me",
      requests: 7,
    });
  });
  it("normalizes URL controls and validates the 31-day custom limit", () => {
    expect(normalizeAdminUsageSearch({})).toMatchObject({
      period: "24h",
      page: 1,
      per_page: 25,
      sort: "requests",
      metric: "tokens",
    });
    expect(
      normalizeAdminUsageSearch({
        page: "2",
        per_page: "50",
        sort: "quantity",
        metric: "images",
      }),
    ).toMatchObject({ page: 2, per_page: 50, metric: "images" });
    expect(
      normalizeAdminUsageSearch({
        user: "bad",
        page: -1,
        period: "all",
        per_page: 1000,
      }),
    ).toMatchObject({ user: undefined, page: 1, period: "24h", per_page: 25 });
    expect(
      usageRangeError("2026-01-01T00:00:00Z", "2026-02-01T00:00:00Z"),
    ).toBeNull();
    expect(
      usageRangeError("2026-01-01T00:00:00Z", "2026-02-01T00:00:01Z"),
    ).toContain("31 days");
    expect(
      usageRangeError("2026-02-01T00:00:00Z", "2026-01-01T00:00:00Z"),
    ).toContain("positive");
    expect(usageRangeError(undefined, "bad")).toBeTruthy();
  });
});
