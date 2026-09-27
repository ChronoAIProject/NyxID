import { expect, it } from "vitest";
import {
  analyticsPath,
  EMPTY_FILTERS,
  filterError,
  formatAnalyticsValue,
  newPanel,
  newView,
  TEMPLATE_COPY,
  duplicatePanel,
  panelSpan,
  partialBucket,
  bucketBounds,
  timeBucketStart,
} from "./usage-analytics";
import {
  SAMPLE_OPTIONS,
  sampleAnalytics,
} from "@/components/billing-analytics/sample-data";
import {
  analyticsPanelSchema,
  analyticsResponseSchema,
  workspaceConfigSchema,
} from "@/schemas/usage-analytics";

it("preserves old panel widths and validates persisted sizing", () => {
  expect(panelSpan(newPanel())).toBe(1);
  expect(panelSpan(newPanel({ wide: true }))).toBe(3);
  expect(panelSpan(newPanel({ wide: true, span: 2 }))).toBe(2);
  expect(
    analyticsPanelSchema.safeParse({ ...newPanel(), span: 4 }).success,
  ).toBe(false);
});

it("aligns calendar intervals in UTC and marks clipped edge buckets", () => {
  expect(
    new Date(
      timeBucketStart(Date.parse("2024-03-03T23:59:59Z"), "week"),
    ).toISOString(),
  ).toBe("2024-02-26T00:00:00.000Z");
  expect(
    new Date(bucketBounds("2024-02-01T00:00:00Z", "month").end).toISOString(),
  ).toBe("2024-03-01T00:00:00.000Z");
  const data = sampleAnalytics(EMPTY_FILTERS, newPanel({ interval: "week" }));
  expect(partialBucket(data.points[0]!.bucket, data)).toBe(true);
});

it("keeps totals and provider token classes consistent across intervals", () => {
  const baseline = sampleAnalytics(EMPTY_FILTERS, newPanel());
  for (const interval of ["hour", "day", "week", "month"] as const) {
    const cost = sampleAnalytics(EMPTY_FILTERS, newPanel({ interval }));
    expect(cost.total).toBe(baseline.total);
    expect(cost.points.reduce((sum, point) => sum + point.value!, 0)).toBe(
      cost.total,
    );
    const input = sampleAnalytics(
      EMPTY_FILTERS,
      newPanel({ interval, measure: "prompt_tokens" }),
    );
    const output = sampleAnalytics(
      EMPTY_FILTERS,
      newPanel({ interval, measure: "completion_tokens" }),
    );
    const total = sampleAnalytics(
      EMPTY_FILTERS,
      newPanel({ interval, measure: "total_tokens" }),
    );
    expect(input.unit).toBe("tokens");
    expect(input.total! + output.total!).toBe(total.total);
    for (let i = 0; i < total.points.length; i++)
      expect(input.points[i]!.value! + output.points[i]!.value!).toBe(
        total.points[i]!.value,
      );
    expect(analyticsPath(EMPTY_FILTERS, newPanel({ interval }))).toContain(
      `interval=${interval}`,
    );
  }
});

it("starts with Operations and persists large boards without a panel count limit", () => {
  const view = newView();
  expect(view.layout).toBe("operations");
  expect(TEMPLATE_COPY.operations.recommended).toBe(true);
  expect(TEMPLATE_COPY.overview.recommended).toBe(false);
  expect(TEMPLATE_COPY.explorer.recommended).toBe(false);
  expect(view.panels).toHaveLength(6);
  view.panels = Array.from({ length: 1000 }, () => newPanel());
  const config = { version: 1, draft: view, saved_views: [] };
  expect(workspaceConfigSchema.safeParse(config).success).toBe(true);
  const tooLarge = workspaceConfigSchema.safeParse({
    ...config,
    saved_views: Array.from({ length: 10 }, () => ({
      ...view,
      id: crypto.randomUUID(),
    })),
  });
  expect(tooLarge.success).toBe(false);
  if (!tooLarge.success)
    expect(tooLarge.error.issues[0]?.message).toContain("1 MiB");
});

it("duplicates Unicode panel titles within the persisted byte limit", () => {
  const original = newPanel({ title: "計".repeat(33) });
  const copy = duplicatePanel(original);
  expect(copy.id).not.toBe(original.id);
  expect(copy.title).toMatch(/ copy$/);
  expect(analyticsPanelSchema.safeParse(copy).success).toBe(true);
});

it("keeps filter keys canonical and separates actors from billing owners", () => {
  const panel = newPanel();
  const one = analyticsPath(
    {
      ...EMPTY_FILTERS,
      services: ["b", "a"],
      actors: ["actor"],
      owners: ["org"],
    },
    panel,
  );
  const two = analyticsPath(
    {
      ...EMPTY_FILTERS,
      services: ["a", "b", "a"],
      actors: ["actor"],
      owners: ["org"],
    },
    panel,
  );
  expect(one).toBe(two);
  expect(one).toContain("actors=actor&owners=org");
});
it("validates custom bounds and never turns missing cost into zero", () => {
  expect(filterError({ ...EMPTY_FILTERS, period: "custom" })).toBeTruthy();
  expect(
    filterError({
      ...EMPTY_FILTERS,
      period: "custom",
      from: "2026-01-01T00:00:00Z",
      to: "2026-02-02T00:00:00Z",
    }),
  ).toBeTruthy();
  expect(formatAnalyticsValue(null, "microcredits")).toBe("Unknown");
  expect(formatAnalyticsValue(1, "microcredits")).toBe("0.000001");
});
it("uses complete sample populations for Top 5/10, time series, and aggregate", () => {
  for (const top of [0, 5, 10] as const) {
    const data = analyticsResponseSchema.parse(
      sampleAnalytics(EMPTY_FILTERS, newPanel({ top })),
    );
    expect(data.slices.reduce((sum, slice) => sum + slice.value!, 0)).toBe(
      data.total,
    );
    expect(data.points.reduce((sum, point) => sum + point.value!, 0)).toBe(
      data.total,
    );
    expect(data.slices.length).toBe(top === 0 ? 1 : top + 1);
  }
});
it("filters sample services, organizations, and users rather than relabeling unchanged data", () => {
  const panel = newPanel({ measure: "quantity", metric: "images" });
  const all = sampleAnalytics(EMPTY_FILTERS, panel);
  const openai = sampleAnalytics(
    { ...EMPTY_FILTERS, services: [SAMPLE_OPTIONS.services[0]!.id] },
    panel,
  );
  expect(all.total).toBeGreaterThan(0);
  expect(openai.total).toBe(0);
  const person = sampleAnalytics(
    { ...EMPTY_FILTERS, actors: [SAMPLE_OPTIONS.actors[0]!.id] },
    newPanel({ measure: "requests" }),
  );
  expect(person.totals.unique_users).toBe(1);
  const org = sampleAnalytics(
    { ...EMPTY_FILTERS, owners: [SAMPLE_OPTIONS.owners[0]!.id] },
    newPanel(),
  );
  expect(org.total).toBeLessThan(
    sampleAnalytics(EMPTY_FILTERS, newPanel()).total!,
  );
});
