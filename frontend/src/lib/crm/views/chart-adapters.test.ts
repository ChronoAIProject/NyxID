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
        key: "qualified",
        label: "Qualified (archived)",
        value: 125_000,
        archived: true,
      },
      { key: "__none__", label: "None", value: 500, archived: false },
      {
        key: "__other__",
        label: "Other",
        value: 250,
        archived: false,
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
