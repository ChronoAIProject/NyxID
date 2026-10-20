import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it } from "vitest";
import { newPanel } from "@/lib/usage-analytics";
import type { AnalyticsResult } from "@/schemas/usage-analytics";
import { usageStats } from "@/test/admin-usage-fixture";
import { ChartView } from "./chart-view";

const data: AnalyticsResult = {
  window: {
    from: "2026-09-18T00:00:00Z",
    to: "2026-09-19T00:00:00Z",
    period: "24h",
  },
  freshness: {
    rolled_up_through: "2026-09-19T00:00:00Z",
    tail_rows: 0,
    validated: true,
  },
  granularity: "hour",
  unit: "requests",
  total: 7,
  totals: usageStats(),
  points: [],
  series: [],
  slices: [
    {
      id: "service-1",
      label: "Example model",
      value: 7,
      unknown_cost_events: 0,
      is_other: false,
    },
  ],
};

it("shows the plotted data table by default", () => {
  render(
    <ChartView
      data={data}
      panel={newPanel({
        title: "Request traffic",
        chart: "bar",
        measure: "requests",
      })}
    />,
  );
  const table = screen.getByRole("table", { name: "Request traffic data" });
  expect(table).toBeVisible();
  expect(within(table).getByText("Example model")).toBeVisible();
  expect(screen.queryByText("View data table")).not.toBeInTheDocument();
});

it("opens and closes the data table in accordion mode", async () => {
  render(
    <ChartView
      data={data}
      panel={newPanel({
        title: "Request traffic",
        chart: "bar",
        measure: "requests",
        table_display: "accordion",
      })}
    />,
  );
  const table = screen.getByRole("table", {
    name: "Request traffic data",
    hidden: true,
  });
  const toggle = screen.getByText("Data table");
  expect(table).not.toBeVisible();
  await userEvent.click(toggle);
  expect(table).toBeVisible();
  await userEvent.click(toggle);
  expect(table).not.toBeVisible();
});
