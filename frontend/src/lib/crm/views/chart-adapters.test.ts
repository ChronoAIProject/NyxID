import { describe, expect, it } from "vitest";
import {
  aggregateResultToChartPoints,
  formatAggregateValue,
} from "@/lib/crm/views/chart-adapters";
import { crmAggregateResultSchema } from "@/lib/crm/views/schema";

describe("aggregateResultToChartPoints", () => {
  it("maps literal aggregate points to literal Recharts data", () => {
    expect(
      aggregateResultToChartPoints({
        truncated: true,
        points: [
          {
            key: "qualified",
            label: "Qualified",
            value: 125_000,
            archived: true,
          },
          { key: null, label: "None", value: 500, archived: false },
          {
            key: "__other__",
            label: "Other",
            value: 250,
            archived: false,
          },
        ],
      }),
    ).toEqual([
      {
        key: "string:qualified",
        label: "Qualified (archived)",
        value: 125_000,
        archived: true,
      },
      { key: "null", label: "None", value: 500, archived: false },
      {
        key: "remainder",
        label: "Other",
        value: 250,
        archived: false,
      },
    ]);
  });

  it("distinguishes literal sentinel values from null and the trailing remainder", () => {
    const result = crmAggregateResultSchema.parse({
      points: [
        { key: null, label: "None", value: 1, archived: false },
        { key: "__none__", label: "Literal none", value: 2, archived: false },
        { key: "__other__", label: "Literal other", value: 3, archived: true },
        { key: "__other__", label: "Other", value: 4, archived: false },
      ],
      unit: { format: "count" },
      total: 10,
      truncated: true,
    });
    const original = structuredClone(result);

    const points = aggregateResultToChartPoints(result);

    expect(points.map((point) => point.key)).toEqual([
      "null",
      "string:__none__",
      "string:__other__",
      "remainder",
    ]);
    expect(new Set(points.map((point) => point.key)).size).toBe(4);
    expect(points.map((point) => point.label)).toEqual([
      "None",
      "Literal none",
      "Literal other (archived)",
      "Other",
    ]);
    expect(result).toEqual(original);
  });

  it("keeps an untruncated final literal Other group as a real archived group", () => {
    expect(
      aggregateResultToChartPoints({
        points: [
          {
            key: "__other__",
            label: "Literal other",
            value: 3,
            archived: true,
          },
        ],
        truncated: false,
      }),
    ).toEqual([
      {
        key: "string:__other__",
        label: "Literal other (archived)",
        value: 3,
        archived: true,
      },
    ]);
  });

  it("labels boolean groups and preserves absent measurements", () => {
    const result = crmAggregateResultSchema.parse({
      points: [
        { key: true, label: "true", value: 3, archived: false },
        { key: false, label: "false", value: null, archived: false },
      ],
      unit: { format: "number" },
      total: null,
      truncated: false,
    });

    expect(aggregateResultToChartPoints(result)).toEqual([
      {
        key: "boolean:true",
        label: "Yes",
        value: 3,
        archived: false,
      },
      {
        key: "boolean:false",
        label: "No",
        value: null,
        archived: false,
      },
    ]);
    expect(formatAggregateValue(result.total, result.unit)).toBe("No value");
  });
});

describe("formatAggregateValue", () => {
  it("formats currency aggregate values from minor units", () => {
    expect(
      formatAggregateValue(125_000, {
        format: "currency",
        currency: "USD",
      }),
    ).toMatch(/1,250\.00/);
  });
});
