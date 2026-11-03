import { describe, expect, it } from "vitest";
import {
  aggregateResultToChartPoints,
  formatAggregateValue,
} from "@/lib/crm/views/chart-adapters";

describe("aggregateResultToChartPoints", () => {
  it("maps literal aggregate points to literal Recharts data", () => {
    expect(
      aggregateResultToChartPoints({
        points: [
          { key: "qualified", label: "Qualified", value: 125_000 },
          { key: null, label: "None", value: 500 },
        ],
      }),
    ).toEqual([
      { key: "qualified", label: "Qualified", value: 125_000 },
      { key: "__none__", label: "None", value: 500 },
    ]);
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
